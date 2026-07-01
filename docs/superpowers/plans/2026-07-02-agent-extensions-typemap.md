# Agent::extensions TypeMap 重构 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `Agent::extensions` 从 `Arc<DashMap<String, Box<dyn Any + Send + Sync>>>` 重构为基于 `TypeId` 的 TypeMap(`Extensions` 类型),消除字符串 key 与每处显式 `downcast`,保留现有共享语义与并发行为。

**Architecture:** 在 `nekocode-core` 新增 `Extensions` 类型,内部仍用 `DashMap<TypeId, Box<dyn Any + Send + Sync>>`,对外暴露 `insert::<T>(v: Arc<T>)` / `get::<T>() -> Option<Arc<T>>` / `remove::<T>() -> Option<Arc<T>>` 三个 typed accessor。key 取 `TypeId::of::<Arc<T>>()`(A 路线),与现有"存 `Arc<T>`"完全一致。`Agent::extensions` 字段类型改为 `Extensions`;所有 publisher / consumer / 构造点三角形替换;删除 `SUBAGENT_EXTENSION_KEY`、`SUBTHREAD_EXTENSION_KEY` 两个字符串常量。

**Tech Stack:** Rust, dashmap(已依赖), std `TypeId` / `Any`, nekocode-core / shell / subagent / subthread。

---

## File Structure

- **Create:** `crates/nekocode-core/src/extensions.rs` — `Extensions` 类型与 typed accessor。
- **Modify:** `crates/nekocode-core/src/lib.rs` — `pub mod extensions;`。
- **Modify:** `crates/nekocode-core/src/agent/mod.rs:59` — `Agent::extensions` 字段类型 `Arc<DashMap<...>>` → `Extensions`;调整 import。
- **Modify:** `crates/nekocode-core/src/agent/new_agent.rs:312` — 测试 helper `make_agent` 的 `extensions` 字面量。
- **Modify:** `crates/nekocode-shell/src/lib.rs:1-49` — `Shell::new` 形参类型与 `insert` 调用;清理 import。
- **Modify:** `crates/nekocode-subagent/src/lib.rs:30` — 删 `SUBAGENT_EXTENSION_KEY`。
- **Modify:** `crates/nekocode-subagent/src/middleware.rs:53,70-73` — `SubagentMiddleware::new` 形参类型与 `insert` 调用;清理 import。
- **Modify:** `crates/nekocode-subagent/src/factory.rs:18` — trait `SubagentMiddlewareFactory::build` 形参类型;清理 import。
- **Modify:** `crates/nekocode-subagent/src/tool/spawn_subagent.rs:104,112,136,155` — `let child_extensions = ...` 改 `Extensions::new()`;透传。
- **Modify:** `crates/nekocode-subagent/src/runner.rs:148` — 测试 helper `make_child` 的 `extensions` 字面量。
- **Modify:** `crates/nekocode-subagent/tests/integration.rs:27` — `MockFactory::build` 形参类型。
- **Modify:** `crates/nekocode-subthread/src/middleware.rs:21,37,48-51` — 删 `SUBTHREAD_EXTENSION_KEY`;`SubthreadMiddleware::new` 形参类型与 `insert` 调用;清理 import。
- **Modify:** `crates/nekocode-subthread/tests/integration.rs:15,57,159,178-184` — 删 key import;`NoopController::activate` Agent 字面量;`build_tools` 的 `extensions` 局部变量与 `.get(...)` 链。
- **Modify:** `crates/nekocode/src/api/thread/mod.rs:31` — `MiddlewareBuildContext::extensions` 字段类型。
- **Modify:** `crates/nekocode/src/api/thread/subagent_factory.rs:25` — `ApiSubagentMiddlewareFactory::build` 形参类型。
- **Modify:** `crates/nekocode/src/api/thread/activate.rs:47` — 局部 `let extensions = ...` 改 `Extensions::new()`。
- **Modify:** `crates/nekocode/src/api/thread/subthread_controller.rs:51` — 局部 `let extensions = ...` 改 `Extensions::new()`。
- **Modify:** `crates/nekocode/src/api/thread/delete.rs:125-137,156-168` — `abort_subthread_tasks` / `abort_subagent_tasks` 的 `.get(key).and_then(downcast)` 链塌缩为 `.get::<T>()`。
- **Modify:** `crates/nekocode/src/api/middleware/shell/list.rs:24-34` — `list_shells` 的连锁 `.get("shell")...downcast...clone()` 塌缩为 `.get::<DashMap<u32, ShellTaskState>>()`。

---

## Task 1: 新增 `Extensions` 类型(nekocode-core)

**目标:** 引入 TypeMap 容器并 export,后续 task 才有类型可用。这一步不修改 `Agent::extensions` 字段类型,只新增模块并 export,确保 cargo check 仍能过(新增类型不会破坏任何现有调用)。

**Files:**
- Create: `crates/nekocode-core/src/extensions.rs`
- Modify: `crates/nekocode-core/src/lib.rs`(加 `pub mod extensions;`)

- [ ] **Step 1: 写 `Extensions`**

创建 `crates/nekocode-core/src/extensions.rs`:

```rust
//! Type-keyed extension map for `Agent`.
//!
//! `Extensions` is a `TypeId`-keyed map (replacing the old `DashMap<String,
//! Box<dyn Any + Send + Sync>>`): each extension is stored as an `Arc<T>` and
//! addressed by `TypeId::of::<Arc<T>>()`. This drops the string-key
//! indirection and the per-call `downcast_ref::<Arc<T>>().cloned()` chain at
//! every reader, while preserving the existing "share one `Arc<T>` across
//! publisher + multiple readers" semantics (the inner map is itself behind an
//! `Arc`, so `Extensions` clones share the same storage, exactly like the old
//! `Arc<DashMap<...>>` field).
//!
//! Convention: publishers always store an `Arc<T>`; readers always read back
//! an `Arc<T>` of the same `T`. There is no support for storing a bare `T`.

use std::any::{Any, TypeId};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Extensions(Arc<dashmap::DashMap<TypeId, Box<dyn Any + Send + Sync>>>);

impl Extensions {
    pub fn new() -> Self {
        Self(Arc::new(dashmap::DashMap::new()))
    }

    /// Insert (or replace) an extension of type `Arc<T>`. Subsequent
    /// `get::<T>()` calls return the latest inserted `Arc<T>`.
    pub fn insert<T: Send + Sync + 'static>(&self, value: Arc<T>) {
        self.0.insert(TypeId::of::<Arc<T>>(), Box::new(value));
    }

    /// Read the extension of type `Arc<T>`, cloning the stored `Arc<T>`.
    /// `None` if no extension of that type has been inserted.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.0
            .get(&TypeId::of::<Arc<T>>())
            .and_then(|b| b.downcast_ref::<Arc<T>>())
            .cloned()
    }

    /// Remove and return the extension of type `Arc<T>`, if present.
    pub fn remove<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.0
            .remove(&TypeId::of::<Arc<T>>())
            .and_then(|(_, b)| b.downcast::<Arc<T>>().ok())
            .map(|boxed| *boxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_roundtrip_returns_same_arc() {
        let ext = Extensions::new();
        let registry = Arc::new(vec![1u32, 2, 3]);
        ext.insert(registry.clone());
        let got = ext.get::<Vec<u32>>().expect("inserted value present");
        assert!(Arc::ptr_eq(&got, &registry));
    }

    #[test]
    fn get_returns_none_when_empty() {
        let ext = Extensions::new();
        assert!(ext.get::<Vec<u32>>().is_none());
    }

    #[test]
    fn remove_returns_inserted_arc_and_clears_slot() {
        let ext = Extensions::new();
        let registry = Arc::new(vec![1u32]);
        ext.insert(registry.clone());
        let removed = ext.remove::<Vec<u32>>().expect("removed inserted value");
        assert!(Arc::ptr_eq(&removed, &registry));
        assert!(ext.get::<Vec<u32>>().is_none());
    }

    #[test]
    fn distinct_types_do_not_collide() {
        let ext = Extensions::new();
        ext.insert(Arc::new(7u32));
        ext.insert(Arc::new(9i64));
        assert_eq!(*ext.get::<u32>().unwrap(), 7);
        assert_eq!(*ext.get::<i64>().unwrap(), 9);
    }

    #[test]
    fn insert_replaces_previous_value_of_same_type() {
        let ext = Extensions::new();
        ext.insert(Arc::new(1u32));
        ext.insert(Arc::new(2u32));
        assert_eq!(*ext.get::<u32>().unwrap(), 2);
    }

    #[test]
    fn clone_shares_storage() {
        let ext = Extensions::new();
        let clone = ext.clone();
        ext.insert(Arc::new(42u32));
        // Same Arc visible through the clone → storage is shared.
        assert!(Arc::ptr_eq(
            &clone.get::<u32>().unwrap(),
            &ext.get::<u32>().unwrap()
        ));
    }
}
```

- [ ] **Step 2: 在 `nekocode-core/src/lib.rs` 加模块导出**

修改 `crates/nekocode-core/src/lib.rs`,在现有 `pub mod agent;` 之前(或之后,任意位置)加一行:

```rust
pub mod extensions;
```

最终该文件头部应为:

```rust
pub mod agent;
pub mod extensions;
pub mod middleware;
pub mod provider;
pub mod types;
```

- [ ] **Step 3: 跑 `Extensions` 单元测试**

Run:
```bash
cargo test -p nekocode-core extensions
```
Expected: PASS,7 个 test(包含上面 6 个 `#[test]`)全过。

- [ ] **Step 4: 确认 workspace 仍能整体编译**

Run:
```bash
cargo check --workspace
```
Expected: 编译成功(新增的 `Extensions` 类型尚未被 `Agent` 字段使用,任何现有调用都不受影响)。

- [ ] **Step 5: Commit**

```bash
git add crates/nekocode-core/src/extensions.rs crates/nekocode-core/src/lib.rs
git commit -m "feat(core): Extensions TypeMap keyed by TypeId::of::<Arc<T>>()"
```

---

## Task 2: `Agent::extensions` 字段类型迁移

**目标:** 把 `Agent::extensions` 的字段类型从 `Arc<DashMap<String, Box<dyn Any + Send + Sync>>>` 改成 `Extensions`,并同步修改所有构造 `Agent { ... }` 字面量的位置。完成后 workspace 不一定能全过(publisher/consumer 还在用旧 API),但 `nekocode-core` 自身要能编。

**Files:**
- Modify: `crates/nekocode-core/src/agent/mod.rs:6-7,59`
- Modify: `crates/nekocode-core/src/agent/new_agent.rs:312`

- [ ] **Step 1: 改 `Agent::extensions` 字段类型与 import**

修改 `crates/nekocode-core/src/agent/mod.rs`:

第 6 行原:
```rust
use std::{any::Any, sync::Arc};
```
改为:
```rust
use std::sync::Arc;
```

在第 7 行 `use std::borrow::Cow;` 之前(或后,保持字母序无所谓)新增:
```rust
use crate::extensions::Extensions;
```

第 59 行原:
```rust
    pub extensions: Arc<dashmap::DashMap<String, Box<dyn Any + Send + Sync>>>,
```
改为:
```rust
    pub extensions: Extensions,
```

- [ ] **Step 2: 改 `nekocode-core` 测试 helper**

修改 `crates/nekocode-core/src/agent/new_agent.rs` 第 306-313 行 `make_agent` 函数。原:
```rust
        Agent {
            thread_id: 0,
            working_directory: "/tmp".into(),
            db,
            middlewares: Arc::new(middlewares),
            provider,
            extensions: Arc::new(dashmap::DashMap::new()),
        }
```
改为:
```rust
        Agent {
            thread_id: 0,
            working_directory: "/tmp".into(),
            db,
            middlewares: Arc::new(middlewares),
            provider,
            extensions: Extensions::new(),
        }
```

(`new_agent.rs` 顶部 `use` 里若有 `dashmap` 仅供此行使用,删 import 若有则会成 unused;`Extensions` 需要导入。先看顶部 use 块再决定。)

查看 `crates/nekocode-core/src/agent/new_agent.rs` 顶部 1-24 行,确认没有裸 `use dashmap`,只有 `use super::*` 之类——其实该文件并不引 dashmap,字段是用全路径 `dashmap::DashMap::new()` 写的。改为 `Extensions::new()` 后,需要在 import 里加 `use crate::extensions::Extensions;`。如果该文件用的是 `use super::*`(就引用 agent/mod 的 re-export)则在 mod.rs 里 `use crate::extensions::Extensions;` 已在父模块作用域,但子模块默认看不到父模块 use——所以稳妥起见本文件明确加:

在 `crates/nekocode-core/src/agent/new_agent.rs` 顶部 use 区(20-24 行附近,跟 `use crate::agent::...` 同处)加:
```rust
use crate::extensions::Extensions;
```

- [ ] **Step 3: 检查 `nekocode-core` 编译**

Run:
```bash
cargo check -p nekocode-core
```
Expected: PASS。`nekocode-core` 内部测试不再用 DashMap 字面量,且没有任何 publisher/consumer 在 `nekocode-core` 里——core 自身能编。

如果报 `Extensions` 未找到,检查 `lib.rs` `pub mod extensions;` 与本文件 `use crate::extensions::Extensions;` 是否都到位。

- [ ] **Step 4: 跑 `nekocode-core` 全部测试**

Run:
```bash
cargo test -p nekocode-core
```
Expected: PASS(包含 Task 1 的 6 个 `Extensions` 单测 + 原有 `run_loop` 等测试)。

- [ ] **Step 5: Commit**

```bash
git add crates/nekocode-core/src/agent/mod.rs crates/nekocode-core/src/agent/new_agent.rs
git commit -m "refactor(core): Agent::extensions field is now Extensions"
```

此时 `cargo check --workspace` 会大量报错(publisher/consumer 用 Δ `Arc<DashMap<String, ...>>` 形参、字符串 key 调用 `.get("shell")` 等),这是预期的——下面任务 3-9 会逐处补上。不要在此步骤强求全 workspace 过。

---

## Task 3: `nekocode-shell` publisher 改造

**目标:** `Shell::new` 形参类型改 `Extensions`,`insert("shell", ...)` 改 typed `insert`;清理 import。完成后 `nekocode-shell` 能编。

**Files:**
- Modify: `crates/nekocode-shell/src/lib.rs:1-55`

- [ ] **Step 1: 修改 import**

原 `crates/nekocode-shell/src/lib.rs` 第 1-7 行:
```rust
use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicUsize},
    },
};
```
`any::Any` 不再需要。改为:
```rust
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicUsize},
    },
};
```

第 9 行 `use nekocode_core::middleware::Middleware;` 之后新增一行(其它 use 按字母/任意序排):
```rust
use nekocode_core::extensions::Extensions;
```

注意:`extensions.insert(shell_states.clone())` 的 `Arc<DashMap<u32, ShellTaskState>>` 仍需 `Arc` 与 `dashmap`(后者通过 `nekocode_core` 间接也行,但 `ShellTaskState` 字段中有 `Arc<AtomicOwned<...>>`、`DashMap` 等已用全路径,本文件其他地方可能仍直接用 `dashmap::DashMap`(如 `Shell::shell_states: Arc<dashmap::DashMap<u32, ShellTaskState>>`)。`dashmap` 在 shell 自己的 `Cargo.toml` 中是否直依?查 `crates/nekocode-shell/Cargo.toml`:

```bash
grep -n dashmap crates/nekocode-shell/Cargo.toml
```

若 shell 直依 dashmap(很可能因为 `shell_states` 字段),保留。`nekocode_core::extensions::Extensions` 与 `nekocode_core::middleware::Middleware` 已通过 necocode-shell 对 nekocode-core 的间接依赖可用,但本文件用了 `nekocode_core::middleware::Middleware` 全路径,说明 shell 已直依 `nekocode-core`(查 Cargo: `grep nekocode-core crates/nekocode-shell/Cargo.toml`)——若确实直依,`Extensions` 同样可用;若不是直依则需要补 dep。预计 shell 直依 nekocode-core(因为 import 了 `nekocode_core::middleware::Middleware`)。

- [ ] **Step 2: 改 `Shell::new` 签名与 insert**

原第 44-55 行:
```rust
    pub fn new(
        extensions: Arc<dashmap::DashMap<String, Box<dyn Any + Send + Sync>>>,
        config: config::ShellConfig,
    ) -> Self {
        let shell_states = Arc::new(dashmap::DashMap::new());
        extensions.insert("shell".into(), Box::new(shell_states.clone()));
        Self {
            shell_states,
            config: Arc::new(config),
            next_id: Arc::new(AtomicU32::new(0)),
        }
    }
```
改为:
```rust
    pub fn new(extensions: Extensions, config: config::ShellConfig) -> Self {
        let shell_states = Arc::new(dashmap::DashMap::new());
        extensions.insert(shell_states.clone());
        Self {
            shell_states,
            config: Arc::new(config),
            next_id: Arc::new(AtomicU32::new(0)),
        }
    }
```

`extensions.insert(shell_states.clone())` 推得 `T = DashMap<u32, ShellTaskState>`,内部存 `Arc<T>`,符合 §1 A 路线。

- [ ] **Step 3: 检查 shell 编译**

Run:
```bash
cargo check -p nekocode-shell
```
Expected: PASS。

(`nekocode-shell` 是否有依赖 `nekocode-core` 但编译时还在 workspace 路径里,只要 metadata 通过 workspace 解析,自身应能编译;若失败信息涉及 import,按 Step 1 末尾的检查补 dep。)

- [ ] **Step 4: Commit**

```bash
git add crates/nekocode-shell/src/lib.rs
git commit -m "refactor(shell): Shell::new takes Extensions, typed insert"
```

---

## Task 4: `nekocode-subagent` publisher 改造与 key 删除

**目标:** `SubagentMiddleware::new` 与 `SubagentMiddlewareFactory::build` 形参类型改 `Extensions`;`insert(SUBAGENT_EXTENSION_KEY, ...)` 改 typed `insert`;`spawn_subagent.rs` 的 `child_extensions` 改 `Extensions::new()`;删 `SUBAGENT_EXTENSION_KEY`;清理 import。完成后 `nekocode-subagent` 能编(但 workspace 还不行,因为 API 层 `nekocode` 还未改)。

**Files:**
- Modify: `crates/nekocode-subagent/src/lib.rs:30`
- Modify: `crates/nekocode-subagent/src/middleware.rs`
- Modify: `crates/nekocode-subagent/src/factory.rs`
- Modify: `crates/nekocode-subagent/src/tool/spawn_subagent.rs`
- Modify: `crates/nekocode-subagent/src/runner.rs:148`
- Modify: `crates/nekocode-subagent/tests/integration.rs:27`

- [ ] **Step 1: 删 `SUBAGENT_EXTENSION_KEY`**

修改 `crates/nekocode-subagent/src/lib.rs`,删除第 27-30 行:
```rust
/// Extension key under which a parent agent publishes its
/// `Arc<SubagentRegistry>` into `Agent.extensions`. Per-parent (NOT a
/// process-global singleton).
pub const SUBAGENT_EXTENSION_KEY: &str = "subagent";
```

同时更新第 7 行文档行原文"`Agent.extensions["subagent"]` as an `Arc<SubagentRegistry>`.":改为:
```rust
//! `Agent.extensions` as an `Arc<SubagentRegistry>` (stored under
//! `TypeId::of::<Arc<SubagentRegistry>>()`).
```

- [ ] **Step 2: 改 `SubagentMiddleware::new` 形参与 insert**

修改 `crates/nekocode-subagent/src/middleware.rs`。

第 1-3 行原:
```rust
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
```
改为(去 `dashmap::DashMap`,加 `Extensions`):
```rust
use std::sync::Arc;

use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
```

第 52-58 行 `new` 签名原:
```rust
    pub fn new(
        specs: Vec<MiddlewareSpec>,
        factory: Arc<dyn SubagentMiddlewareFactory>,
        parent_provider: Arc<dyn Provider>,
        parent_extensions: Arc<DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
        parent_db: toasty::Db,
        parent_working_directory: String,
        config: crate::SubagentConfig,
        depth: u32,
        allow_nested: bool,
    ) -> Self {
```
把 `parent_extensions` 那一行改为:
```rust
        parent_extensions: Extensions,
```

第 70-73 行原:
```rust
        parent_extensions.insert(
            crate::SUBAGENT_EXTENSION_KEY.into(),
            Box::new(registry.clone()) as Box<dyn std::any::Any + Send + Sync>,
        );
```
改为:
```rust
        parent_extensions.insert(registry.clone());
```

- [ ] **Step 3: 改 `SubagentMiddlewareFactory::build` 形参**

修改 `crates/nekocode-subagent/src/factory.rs`。第 1-5 行原:
```rust
use std::any::Any;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
```
改为:
```rust
use std::sync::Arc;

use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
```

第 7-11 行文档段尾"`extensions` is the child's fresh DashMap so middleware like shell gets its own session map."更新为"`extensions` is the child's fresh `Extensions` so middleware like shell gets its own session map."

第 17-19 行 trait 方法原:
```rust
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    ) -> Box<dyn Middleware>;
```
改为:
```rust
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Extensions,
    ) -> Box<dyn Middleware>;
```

- [ ] **Step 4: 改 `spawn_subagent.rs` 的 `child_extensions`**

修改 `crates/nekocode-subagent/src/tool/spawn_subagent.rs` 第 104 行:
```rust
        let child_extensions = Arc::new(dashmap::DashMap::new());
```
改为:
```rust
        let child_extensions = Extensions::new();
```

确认本文件顶部 import 含 `use nekocode_core::extensions::Extensions;`(若没有则加)。先看该文件第 1-30 行 import 块,在合适位置加:
```rust
use nekocode_core::extensions::Extensions;
```

后续第 112、136、155 行原本传 `child_extensions.clone()` —— `Extensions: Clone` 共享内部 Arc,`.clone()` 仍是合法的(语义:多个持有者共享同一份 DashMap)。不需改动这些行。其中第 155 行 `extensions: child_extensions,` 直接赋给 `Agent::extensions`(已为 `Extensions` 类型),也已合法。

- [ ] **Step 5: 改 subagent runner 测试 helper**

修改 `crates/nekocode-subagent/src/runner.rs` 第 142-149 行 `make_child` 返回 Agent 字面量。原:
```rust
        Agent {
            thread_id: 0,
            working_directory: "/tmp".into(),
            db,
            middlewares: Arc::new(Vec::new()),
            provider,
            extensions: Arc::new(dashmap::DashMap::new()),
        }
```
改为:
```rust
        Agent {
            thread_id: 0,
            working_directory: "/tmp".into(),
            db,
            middlewares: Arc::new(Vec::new()),
            provider,
            extensions: Extensions::new(),
        }
```

本文件 import 区加(若未有):
```rust
use nekocode_core::extensions::Extensions;
```

- [ ] **Step 6: 改 subagent integration test 的 mock 形参**

修改 `crates/nekocode-subagent/tests/integration.rs` 第 22-31 行:
```rust
struct MockFactory;
#[async_trait::async_trait]
impl SubagentMiddlewareFactory for MockFactory {
    fn build(
        &self,
        _spec: MiddlewareSpec,
        _subagent_id: u64,
        _extensions: Arc<dashmap::DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
    ) -> Box<dyn Middleware> {
        Box::new(NoopMiddleware)
    }
}
```
改为:
```rust
struct MockFactory;
#[async_trait::async_trait]
impl SubagentMiddlewareFactory for MockFactory {
    fn build(
        &self,
        _spec: MiddlewareSpec,
        _subagent_id: u64,
        _extensions: Extensions,
    ) -> Box<dyn Middleware> {
        Box::new(NoopMiddleware)
    }
}
```

测试文件 import 区(第 1-18 行附近)加:
```rust
use nekocode_core::extensions::Extensions;
```

(原 import 行 `use std::sync::{Arc, Mutex};` 中的 `Arc` 仍可能被文件其他地方使用——保留。若 clippy 报 `Arc` unused,后续 Task 10 清理。)

- [ ] **Step 7: subagent 自身编译**

Run:
```bash
cargo check -p nekocode-subagent --tests
```
Expected: PASS。`nekocode-subagent` 三个 publisher 形参与构造点全部用 `Extensions`,自身闭环。

- [ ] **Step 8: subagent 测试**

Run:
```bash
cargo test -p nekocode-subagent
```
Expected: PASS。

- [ ] **Step 9: Commit**

```bash
git add crates/nekocode-subagent/src crates/nekocode-subagent/tests
git commit -m "refactor(subagent): drop SUBAGENT_EXTENSION_KEY, use Extensions"
```

---

## Task 5: `nekocode-subthread` publisher 改造与 key 删除

**目标:** 与 Task 4 对称,但只一处 publisher(`SubthreadMiddleware::new`)与一处测试断言(`build_tools`)需要 consumer 形态。

**Files:**
- Modify: `crates/nekocode-subthread/src/middleware.rs`
- Modify: `crates/nekocode-subthread/tests/integration.rs`

- [ ] **Step 1: 改 import 与删 key**

修改 `crates/nekocode-subthread/src/middleware.rs`。第 1-6 行原:
```rust
use std::any::Any;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::Middleware;
use nekocode_types::tool::ToolRegistry;
```
改为:
```rust
use std::sync::Arc;

use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::Middleware;
use nekocode_types::tool::ToolRegistry;
```

删除第 18-21 行:
```rust
/// Extension key under which the per-parent `SubthreadRegistry` is stored on
/// the parent's `Agent.extensions`. The API layer (cascade delete) reads it
/// from there to abort any in-flight subthread tasks.
pub const SUBTHREAD_EXTENSION_KEY: &str = "subthread";
```

第 23-27 行顶部文档段里提到 `SUBTHREAD_EXTENSION_KEY` 的句子(第 25-26 行):
```rust
/// per-parent `SubthreadRegistry`, exposed via `Agent.extensions` under
/// `SUBTHREAD_EXTENSION_KEY` so the API layer can reach it for cascade
```
改为:
```rust
/// per-parent `SubthreadRegistry`, exposed via `Agent.extensions` (typed
/// `Arc<SubthreadRegistry>` slot) so the API layer can reach it for cascade
```

- [ ] **Step 2: 改 `SubthreadMiddleware::new`**

第 32-59 行 `new`。原第 36-42 行:
```rust
    pub fn new(
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
        db: toasty::Db,
        parent_thread_id: u64,
        parent_working_directory: String,
        config: SubthreadConfig,
        controller: Arc<dyn ThreadController>,
    ) -> Self {
```
第一行改为:
```rust
    pub fn new(
        extensions: Extensions,
        db: toasty::Db,
```

第 44-51 行原:
```rust
        let registry = Arc::new(SubthreadRegistry::new());
        // Publish the registry to the agent's extensions so the API layer can
        // reach it (e.g. delete_thread → abort all subthreads). Keep a clone
        // for our own tools' use.
        extensions.insert(
            SUBTHREAD_EXTENSION_KEY.to_string(),
            Box::new(registry.clone()),
        );
```
改为:
```rust
        let registry = Arc::new(SubthreadRegistry::new());
        // Publish the registry to the agent's extensions so the API layer can
        // reach it (e.g. delete_thread → abort all subthreads). Keep a clone
        // for our own tools' use.
        extensions.insert(registry.clone());
```

- [ ] **Step 3: 改 subthread integration test**

修改 `crates/nekocode-subthread/tests/integration.rs`。

第 12-16 行 import 原含:
```rust
use nekocode_subthread::{
    SubthreadConfig, SubthreadMiddleware,
    controller::{ActivationOutcome, ThreadController},
    middleware::SUBTHREAD_EXTENSION_KEY,
};
```
删去 `middleware::SUBTHREAD_EXTENSION_KEY,` 这一行,并加(顶部 import 区,第 1-16 行附近):
```rust
use nekocode_core::extensions::Extensions;
```

第 57 行原:
```rust
                extensions: std::sync::Arc::new(dashmap::DashMap::new()),
```
改为:
```rust
                extensions: Extensions::new(),
```

第 159-160 行原:
```rust
    let extensions: Arc<DashMap<String, Box<dyn std::any::Any + Send + Sync>>> =
        Arc::new(DashMap::new());
```
改为:
```rust
    let extensions = Extensions::new();
```

(注:本文件第 10 行 `use dashmap::DashMap;` —— 在删掉上面那两处 `DashMap` 用法后,检查本文件是否还有其他 `DashMap` 引用。`grep -n DashMap crates/nekocode-subthread/tests/integration.rs` 若仅剩 import,删该 import;若还有其他用处则保留。)

第 178-184 行原:
```rust
    let registry = extensions
        .get(SUBTHREAD_EXTENSION_KEY)
        .and_then(|b| {
            b.downcast_ref::<Arc<nekocode_subthread::SubthreadRegistry>>()
                .cloned()
        })
        .expect("subthread registry published to extensions");
```
改为:
```rust
    let registry = extensions
        .get::<nekocode_subthread::SubthreadRegistry>()
        .expect("subthread registry published to extensions");
```

(`Extensions::get::<T>()` 返回 `Option<Arc<T>>`,此处 `registry` 推得 `Arc<SubthreadRegistry>`,与原变量类型一致。)

- [ ] **Step 4: subthread 自身编译**

Run:
```bash
cargo check -p nekocode-subthread --tests
```
Expected: PASS。

- [ ] **Step 5: subthread 测试**

Run:
```bash
cargo test -p nekocode-subthread
```
Expected: PASS(集成测试 `build_tools` 仍能从 extensions 拽出 `Arc<SubthreadRegistry>`)。

- [ ] **Step 6: Commit**

```bash
git add crates/nekocode-subthread/src/middleware.rs crates/nekocode-subthread/tests/integration.rs
git commit -m "refactor(subthread): drop SUBTHREAD_EXTENSION_KEY, use Extensions"
```

---

## Task 6: API 层 consumer 改造 — cascade delete

**目标:** `delete.rs` 的两处 `.get(key).and_then(downcast)` 链塌缩为 `Extensions::get::<T>()`。

**Files:**
- Modify: `crates/nekocode/src/api/thread/delete.rs:125-137,156-168`

- [ ] **Step 1: 改 `abort_subthread_tasks`**

`crates/nekocode/src/api/thread/delete.rs` 第 125-137 行原:
```rust
    let registry: Option<Arc<nekocode_subthread::SubthreadRegistry>> =
        if let Some(agent_entry) = active_threads.get(&thread_id) {
            let agent = agent_entry.value().read().await;
            agent
                .extensions
                .get(nekocode_subthread::middleware::SUBTHREAD_EXTENSION_KEY)
                .and_then(|b| {
                    b.downcast_ref::<Arc<nekocode_subthread::SubthreadRegistry>>()
                        .cloned()
                })
        } else {
            None
        };
```
改为:
```rust
    let registry: Option<Arc<nekocode_subthread::SubthreadRegistry>> =
        if let Some(agent_entry) = active_threads.get(&thread_id) {
            let agent = agent_entry.value().read().await;
            agent.extensions.get::<nekocode_subthread::SubthreadRegistry>()
        } else {
            None
        };
```

- [ ] **Step 2: 改 `abort_subagent_tasks`**

同文件第 156-168 行原:
```rust
    let registry: Option<Arc<nekocode_subagent::SubagentRegistry>> =
        if let Some(agent_entry) = active_threads.get(&thread_id) {
            let agent = agent_entry.value().read().await;
            agent
                .extensions
                .get(nekocode_subagent::SUBAGENT_EXTENSION_KEY)
                .and_then(|b| {
                    b.downcast_ref::<Arc<nekocode_subagent::SubagentRegistry>>()
                        .cloned()
                })
        } else {
            None
        };
```
改为:
```rust
    let registry: Option<Arc<nekocode_subagent::SubagentRegistry>> =
        if let Some(agent_entry) = active_threads.get(&thread_id) {
            let agent = agent_entry.value().read().await;
            agent.extensions.get::<nekocode_subagent::SubagentRegistry>()
        } else {
            None
        };
```

- [ ] **Step 3: 编译检查(预期未通过,只验证错处不再有 downcast 与 key 引用)**

Run:
```bash
cargo check -p nekocode 2>&1 | head -80
```
Expected: 仍有编译错,但错误类型应来自 `MiddlewareBuildContext::extensions` 字段类型(下一 task)、`activate.rs` / `subthread_controller.rs` 的 `Arc::new(DashMap::new())` 字面量。`delete.rs` 本身对 `SUBAGENT_EXTENSION_KEY` / `downcast_ref` 的引用应已无。

确认无遗留 downcast 报错:
```bash
cargo check -p nekocode 2>&1 | grep -E 'downcast|EXTENSION_KEY|delete.rs' | head
```
Expected: 空(或仅与文件无关的间接错误)。

- [ ] **Step 4: Commit**

```bash
git add crates/nekocode/src/api/thread/delete.rs
git commit -m "refactor(api): cascade delete reads Extensions via typed get::<T>()"
```

---

## Task 7: API 层 consumer 改造 — shell list

**目标:** `list_shells` 的连锁 `.get("shell").ok_or(...)?.downcast_ref::<Arc<...>>().ok_or(...)?.clone()` 塌缩为 typed `get`。

**Files:**
- Modify: `crates/nekocode/src/api/middleware/shell/list.rs:24-34`

- [ ] **Step 1: 改 `list_shells` 第 24-34 行**

原:
```rust
    let shell_states = thread_state
        .read()
        .await
        .extensions
        .get("shell")
        .ok_or_else(|| {
            ApiError::ItemNotFound(String::from("shell middleware not configured for thread"))
        })?
        .downcast_ref::<Arc<dashmap::DashMap<u32, ShellTaskState>>>()
        .ok_or_else(|| ApiError::ItemNotFound(String::from("shell middleware ext")))?
        .clone();
```
改为:
```rust
    let shell_states = thread_state
        .read()
        .await
        .extensions
        .get::<dashmap::DashMap<u32, ShellTaskState>>()
        .ok_or_else(|| {
            ApiError::ItemNotFound(String::from("shell middleware not configured for thread"))
        })?;
```

注意 turbofish 写 **裸** `DashMap<u32, ShellTaskState>`(§1 A 路线对应到 `get::<T> -> Option<Arc<T>>`),shell publisher(Task 3)以 `extensions.insert(shell_states)` 形式存 `Arc<DashMap<u32, ShellTaskState>>`,与本处 `get::<DashMap<u32, ShellTaskState>>()` 的 `TypeId::of::<Arc<...>>()` 完全一致。

后续 34 行后 `let shell_states: Vec<serde_json::Value> = shell_states.iter()...` 不变——`shell_states` 现在是 `Arc<DashMap<u32, ShellTaskState>>`,与原 `Arc` clone 拿到的类型完全相同,后续代码无改动。

- [ ] **Step 2: 检查编译(预期未通过,但本文件已无 downcast)**

Run:
```bash
cargo check -p nekocode 2>&1 | grep -E 'downcast|"shell"|list.rs' | head
```
Expected: 空(list.rs 自身干净)。

- [ ] **Step 3: Commit**

```bash
git add crates/nekocode/src/api/middleware/shell/list.rs
git commit -m "refactor(api/shell): list_shells reads shell_states via typed get"
```

---

## Task 8: API 层构造点与 ctx 字段类型迁移

**目标:** `MiddlewareBuildContext::extensions`、`ApiSubagentMiddlewareFactory::build`、`activate.rs` 的局部 `extensions` 变量、`subthread_controller.rs` 的局部 `extensions` 变量,全部三角形 `Arc<DashMap<...>>` → `Extensions`。完成后 `cargo check --workspace` 全过。

**Files:**
- Modify: `crates/nekocode/src/api/thread/mod.rs:31,59,88,124`
- Modify: `crates/nekocode/src/api/thread/subagent_factory.rs:1-5,25,28-31`
- Modify: `crates/nekocode/src/api/thread/activate.rs:47`
- Modify: `crates/nekocode/src/api/thread/subthread_controller.rs:51,56,84`

- [ ] **Step 1: 改 `MiddlewareBuildContext` 字段与 build_middlewares 调用**

修改 `crates/nekocode/src/api/thread/mod.rs`。

顶部 import(找该文件第 1-4 行):
```rust
use axum::routing::{get, post};
use std::sync::Arc;

use crate::AppState;
```
在其后或合适位置加:
```rust
use nekocode_core::extensions::Extensions;
```

第 28-36 行 `MiddlewareBuildContext`,第 31 行原:
```rust
    pub extensions: Arc<dashmap::DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
```
改为:
```rust
    pub extensions: Extensions,
```

第 55-61 行(`"shell"` 分支)原:
```rust
                middlewares.push(Box::new(nekocode_shell::Shell::new(
                    ctx.extensions.clone(),
                    cfg,
                )));
```
`ctx.extensions.clone()` 现在 `Extensions: Clone`(共享内部 Arc),仍是合法的;无需改。

后续第 88 行 subthread 分支调 `SubthreadMiddleware::new(ctx.extensions.clone(), ...)`,以及第 124 行 subagent 分支调 `SubagentMiddleware::new(..., ctx.extensions.clone(), ...)`,同样 `clone` 还是 `Extensions::clone`,无需改。`Arc::clone` → `Extensions::clone` 行为等价(共享底层 DashMap)。

- [ ] **Step 2: 改 `ApiSubagentMiddlewareFactory::build`**

修改 `crates/nekocode/src/api/thread/subagent_factory.rs`。第 1-6 行原:
```rust
use std::any::Any;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_subagent::SubagentMiddlewareFactory;
```
改为:
```rust
use std::sync::Arc;

use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_subagent::SubagentMiddlewareFactory;
```

第 21-26 行 trait impl 方法签名原:
```rust
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    ) -> Box<dyn Middleware> {
```
改为:
```rust
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Extensions,
    ) -> Box<dyn Middleware> {
```

第 28-31 行 body 内 `extensions.clone()` 仍是合法 `Extensions::clone`,无需改动:
```rust
            "shell" => Box::new(nekocode_shell::Shell::new(
                extensions.clone(),
                nekocode_shell::config::ShellConfig::from_value(&spec.config),
            )),
```

- [ ] **Step 3: 改 `activate.rs`**

修改 `crates/nekocode/src/api/thread/activate.rs`。第 47 行原:
```rust
    let extensions = Arc::new(dashmap::DashMap::new());
```
改为:
```rust
    let extensions = Extensions::new();
```

顶部 import 加(第 1-8 行附近):
```rust
use nekocode_core::extensions::Extensions;
```

(第 81 行 `extensions,` 字段赋给 `Agent::extensions`(已为 `Extensions` 类型),无需改。)

- [ ] **Step 4: 改 `subthread_controller.rs`**

修改 `crates/nekocode/src/api/thread/subthread_controller.rs`。第 51 行原:
```rust
        let extensions = Arc::new(dashmap::DashMap::new());
```
改为:
```rust
        let extensions = Extensions::new();
```

本文件 import 区加:
```rust
use nekocode_core::extensions::Extensions;
```

(第 56、84 行 `extensions.clone()`、`extensions,` 都是赋给 `MiddlewareBuildContext::extensions` 或 `Agent::extensions`,均为 `Extensions`,无需改。)

- [ ] **Step 5: workspace 全量编译**

Run:
```bash
cargo check --workspace
```
Expected: PASS。所有 publisher/consumer/构造点全部用 `Extensions`。

- [ ] **Step 6: Commit**

```bash
git add crates/nekocode/src/api/thread/mod.rs crates/nekocode/src/api/thread/subagent_factory.rs crates/nekocode/src/api/thread/activate.rs crates/nekocode/src/api/thread/subthread_controller.rs
git commit -m "refactor(api): MiddlewareBuildContext/factory/activate use Extensions"
```

---

## Task 9: clippy 收尾与文档残留扫描

**目标:** 扫一遍 `unused_imports`、`dead_code`,清理掉因三角形替换而失引用的 import(`Any`、`DashMap`、key 常量等);确认无残留字符串 key 或 downcast 链。

**Files:** 视扫描结果而定(workspace 内任意 .rs)。

- [ ] **Step 1: clippy 全 workspace**

Run:
```bash
cargo clippy --workspace --all-targets 2>&1 | tee /tmp/clippy.log
```
Expected: PASS,可能有 warning 列表。重点看:
- `unused import: std::any::Any` / `dashmap::DashMap` / `nekocode_subagent::SUBAGENT_EXTENSION_KEY` / `nekocode_subthread::middleware::SUBTHREAD_EXTENSION_KEY`
- `unused import: Arc` / `Mutex`(只用在已删行)

- [ ] **Step 2: 逐文件清 unused import**

按 `/tmp/clippy.log` 报告,逐文件 Edit 删除对应 import 行。常见位置:
- `crates/nekocode-subthread/tests/integration.rs` 第 10 行 `use dashmap::DashMap;` —— 若 grep 确认本文件无其他 `DashMap` 形参/字面量,删。
- `crates/nekocode-subagent/tests/integration.rs` —— `Arc` 是否仍被用(`MockProvider` 等),若仍用则保留;只删真正 unused 的项。
- `crates/nekocode-shell/src/lib.rs` —— 已在 Task 3 删 `Any`,再确认无残留 `DashMap` 或 `Box` 之类。

清完后:
```bash
cargo clippy --workspace --all-targets 2>&1 | grep -E 'warning|error' | head -20
```
Expected: 空(或与本 task 无关的既有 warning)。

- [ ] **Step 3: 残留字符串 key / downcast 扫描**

确认全仓再无字符串 key 或 `downcast_ref::<Arc<...>>()` 用在 extensions 上:
```bash
grep -rn 'extensions' --include="*.rs" . | grep -E '"shell"|"subagent"|"subthread"|downcast_ref' | grep -v '//' | head -20
```
Expected: 空。

确认 key 常量已删:
```bash
grep -rn 'EXTENSION_KEY' --include="*.rs" . | head
```
Expected: 空(若有提及均为注释/doc,人工核对一次)。

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: drop unused imports after Extensions refactor"
```

---

## Task 10: 整体验证

**目标:** 所有测试通过,确认重构无回归。

- [ ] **Step 1: 全 workspace 测试**

Run:
```bash
cargo test --workspace
```
Expected: PASS。重点验:
- `nekocode-core::extensions::tests::*`(6 个)
- `nekocode-subagent`(集成测试 + runner)
- `nekocode-subthread::integration::*`(尤其 `build_tools` 能从 Extensions 拽出 `Arc<SubthreadRegistry>`)
- `nekocode-core::agent::new_agent::tests::*`(run_loop 相关)

- [ ] **Step 2: 若任一测试失败,定位并修(回到对应 task)**

常见失败模式:
- `TypeId` 覆盖:两处 publisher 误存同一 `TypeId`,集成测试 `get::<T>()` 返回 None → 检查 `Extensions::insert` 的 `T` 是否唯一。
- `downcast` panic:写在 turbofish 里 `Arc<T>` 与 publisher 存的 `Arc<T>` 不一致 → 比对 publisher 与 consumer 的 `T`。
- 构造时 `extensions` 类型不匹配:`Arc::new(DashMap::new())` 残留 → 全文 grep `DashMap::new()`.

- [ ] **Step 3: 最终 commit(若有 fix)**

```bash
git add -A && git commit -m "test: green after Extensions TypeMap refactor"
```

若无失败,本 task 无 commit。

---

## Self-Review Notes

**Spec coverage:**
- §1 `Extensions` 类型定义 → Task 1。
- §2 三 publisher 改造 → Task 3(shell)、Task 4(subagent)、Task 5(subthread)。
- §3 consumer 改造 → Task 6(delete)、Task 7(shell list),+ Task 5 Step 3(subthread 测试断言,作为 consumer)。
- §4 删 key 常量、形参类型迁移、Agent 字段、构造点 → Task 2(Agent 字段+core helper),Task 4(subagent factory trait),Task 8(API 层 ctx/factory/activate/controller)。
- §5 测试与验证计划 → Task 9(clippy 收尾 + 残留扫描)、Task 10(workspace 全测试)。

**Placeholder scan:** 无 TBD/"add appropriate"等内容;每步都带具体代码片段或具体命令与预期输出。

**Type consistency:** 全文 `Extensions::insert(v: Arc<T>)` / `get::<T>() -> Option<Arc<T>>`,`T` 一律裸类型(`DashMap<u32, ShellTaskState>`、`SubagentRegistry`、`SubthreadRegistry`、`Vec<u32>` 等);`TypeId::of::<Arc<T>>()` 统一在 §1 与 Task 1 实现里出现,后续 task 不再重复定义。turbofish 在 `get::<T>()` 处一律不带 `Arc<>`,与 §1 设计章节 A 路线一致。