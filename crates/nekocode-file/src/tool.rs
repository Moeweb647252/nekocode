use std::sync::Arc;

use nekocode_entities::thread::Thread;
use nekocode_types::tool::{Tool, ToolError};

use crate::config::FileConfig;

/// Read a file's text content, optionally restricted to a 1-based inclusive
/// line range. Useful for paging large files without pulling the whole thing
/// into the model context.
pub struct ReadFileTool {
    pub config: Arc<FileConfig>,
}

#[async_trait::async_trait]
impl Tool for ReadFileTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "read_file".to_string(),
            description: "Read the text content of a file. Optionally return only a 1-based "
                .to_string()
                + "inclusive range of lines via start_line/end_line, which is useful for "
                + "paging large files.",
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read. Relative paths resolve against the configured working directory."
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "First line to return (1-based, inclusive). Optional."
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "Last line to return (1-based, inclusive). Optional. Must be >= start_line."
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let path = parse_path(&params, &self.config)?;
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to read {}: {e}", path.display())))?;

        let resolved = match (parse_optional_u64(&params, "start_line"), parse_optional_u64(&params, "end_line")) {
            (Some(start), Some(end)) => {
                if start == 0 || end < start {
                    return Err(ToolError::InvalidParameters(format!(
                        "Invalid line range: start_line={start}, end_line={end} (both must be >= 1 and end_line >= start_line)"
                    )));
                }
                Some((start, end))
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(ToolError::InvalidParameters(
                    "start_line and end_line must be provided together".into(),
                ));
            }
            (None, None) => None,
        };

        let content = match resolved {
            None => content,
            Some((start, end)) => {
                // 1-based inclusive range. Lines past EOF simply contribute nothing.
                let start = start as usize;
                let end = end as usize;
                let mut out: Vec<&str> = Vec::with_capacity(end.saturating_sub(start).saturating_add(1));
                for (i, line) in content.lines().enumerate() {
                    let line_no = i + 1;
                    if line_no > end {
                        break;
                    }
                    if line_no >= start {
                        out.push(line);
                    }
                }
                out.join("\n")
            }
        };

        Ok(serde_json::json!({
            "path": path.to_string_lossy(),
            "content": content,
        }))
    }
}

/// Create or overwrite a file. Parent directories are created as needed.
pub struct WriteFileTool {
    pub config: Arc<FileConfig>,
}

#[async_trait::async_trait]
impl Tool for WriteFileTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "write_file".to_string(),
            description: "Create or overwrite a file with the given content. Parent directories "
                .to_string()
                + "are created automatically if they do not exist.",
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write. Relative paths resolve against the configured working directory."
                    },
                    "content": {
                        "type": "string",
                        "description": "The full content to write to the file."
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let path = parse_path(&params, &self.config)?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'content' parameter".into()))?;

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::ExecutionError(format!("Failed to create parent dirs for {}: {e}", path.display())))?;
        }

        let bytes = content.len();
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to write {}: {e}", path.display())))?;

        Ok(serde_json::json!({
            "path": path.to_string_lossy(),
            "bytes_written": bytes,
        }))
    }
}

/// Edit a file by replacing an exact substring. By default the match must be
/// unique; set `replace_all` to replace every occurrence.
pub struct EditFileTool {
    pub config: Arc<FileConfig>,
}

#[async_trait::async_trait]
impl Tool for EditFileTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "edit_file".to_string(),
            description: "Edit a file by replacing an exact substring match. By default the old "
                .to_string()
                + "string must occur exactly once; set replace_all to true to replace every "
                + "occurrence. old_string must match exactly including whitespace and indentation.",
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit. Relative paths resolve against the configured working directory."
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to find in the file, including whitespace and indentation. Must differ from new_string."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The text to replace old_string with."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "If true, replace every occurrence of old_string. Defaults to false, which requires exactly one occurrence."
                    }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let path = parse_path(&params, &self.config)?;
        let old_string = params
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'old_string' parameter".into()))?;
        let new_string = params
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'new_string' parameter".into()))?;
        if old_string == new_string {
            return Err(ToolError::InvalidParameters(
                "old_string and new_string must differ".into(),
            ));
        }
        let replace_all = params
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to read {}: {e}", path.display())))?;

        let count = content.matches(old_string).count();
        if count == 0 {
            return Err(ToolError::ExecutionError(format!(
                "old_string was not found in {}. Make sure it matches exactly, including whitespace and indentation.",
                path.display()
            )));
        }
        if count > 1 && !replace_all {
            return Err(ToolError::ExecutionError(format!(
                "old_string occurs {count} times in {}. Provide more context to make it unique, or set replace_all to true.",
                path.display()
            )));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            // Safe to replace_range: we've established exactly one match.
            let idx = content.find(old_string).expect("match count was checked");
            let mut buf = content.clone();
            buf.replace_range(idx..idx + old_string.len(), new_string);
            buf
        };

        tokio::fs::write(&path, &new_content)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to write {}: {e}", path.display())))?;

        Ok(serde_json::json!({
            "path": path.to_string_lossy(),
            "replacements_made": if replace_all { count } else { 1 },
        }))
    }
}

/// Resolve the `path` parameter against the configured working directory and
/// verify it stays within the sandbox boundary.
fn parse_path(params: &serde_json::Value, config: &FileConfig) -> Result<std::path::PathBuf, ToolError> {
    let raw = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidParameters("Missing 'path' parameter".into()))?;
    config.resolve_and_check(raw).map_err(ToolError::InvalidParameters)
}

fn parse_optional_u64(params: &serde_json::Value, key: &str) -> Option<u64> {
    params.get(key).and_then(|v| v.as_u64())
}

/// Set the current thread's title. The model is encouraged to call this
/// once the user's intent is clear so the thread has a meaningful label
/// in the UI.
///
/// Unlike the file tools, this one mutates database state rather than the
/// filesystem, so it carries a cloned `toasty::Db` handle and the owning
/// thread id instead of a `FileConfig`. Both are injected when the thread
/// is activated (`activate_thread`) and threaded through `ToolMiddleware`.
pub struct SetTitleTool {
    pub db: toasty::Db,
    pub thread_id: u64,
}

#[async_trait::async_trait]
impl Tool for SetTitleTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "set_title".to_string(),
            description: "Set the current thread's title. Use a short, descriptive label "
                .to_string()
                + "(a few words) summarizing the thread's topic or current goal. Call this "
                + "once the user's intent becomes clear; subsequent calls overwrite the title.",
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "The new title. Concise and descriptive."
                    }
                },
                "required": ["title"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let raw = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'title' parameter".into()))?;
        let title = raw.trim();
        if title.is_empty() {
            return Err(ToolError::InvalidParameters(
                "title must be a non-empty, non-whitespace string".into(),
            ));
        }
        let title = title.to_string();
        // Clone the (cheap, internally Arc-based) DB handle so each tool call
        // mutates its own local connection, mirroring the pattern used in
        // `Agent::run_loop` (`let mut db = self.db.clone()`).
        let mut db = self.db.clone();
        let mut update = toasty::query!(Thread FILTER .id == #(self.thread_id)).update();
        update.set_title(title.clone());
        update.exec(&mut db).await.map_err(|e| {
            ToolError::ExecutionError(format!(
                "Failed to set thread title for thread {}: {e}",
                self.thread_id
            ))
        })?;
        Ok(serde_json::json!({
            "thread_id": self.thread_id,
            "title": title,
        }))
    }
}

#[cfg(test)]
mod set_title_tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use nekocode_entities::prepare_db;
    use nekocode_entities::thread::Thread;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// Return a unique temp file path for a test DB. Each call yields a new
    /// file so parallel tests don't collide.
    fn test_db_path() -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "nekocode_set_title_test_{}_{}.db",
            std::process::id(),
            n
        ))
    }

    /// End-to-end check that calling `set_title` actually updates the row.
    /// Validates the toasty call shape against a real DB.
    #[tokio::test]
    async fn set_title_updates_thread_row() {
        let mut db = prepare_db(test_db_path())
            .await
            .expect("prepare_db");
        let thread = toasty::create!(Thread {
            working_directory: "/tmp".to_string(),
            model: "default".to_string(),
        })
        .exec(&mut db)
        .await
        .expect("create thread");

        let tool = SetTitleTool {
            db: db.clone(),
            thread_id: thread.id,
        };
        let out = tool
            .call(serde_json::json!({ "title": "  Hello world  " }))
            .await
            .expect("call");
        assert_eq!(out["title"], "Hello world");

        let row = toasty::query!(Thread FILTER .id == #(thread.id))
            .first()
            .exec(&mut db)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.title.as_deref(), Some("Hello world"));
    }

    #[tokio::test]
    async fn set_title_rejects_empty() {
        let mut db = prepare_db(test_db_path())
            .await
            .expect("prepare_db");
        let thread = toasty::create!(Thread {
            working_directory: "/tmp".to_string(),
            model: "default".to_string(),
        })
        .exec(&mut db)
        .await
        .expect("create thread");
        let tool = SetTitleTool {
            db: db.clone(),
            thread_id: thread.id,
        };
        for bad in [serde_json::json!({}), serde_json::json!({ "title": "" }), serde_json::json!({ "title": "   " })] {
            let err = tool.call(bad).await.expect_err("must reject");
            assert!(matches!(err, ToolError::InvalidParameters(_)), "got {err:?}");
        }
    }
}
