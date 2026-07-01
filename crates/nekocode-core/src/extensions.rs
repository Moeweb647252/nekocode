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
            .and_then(|b| b.downcast_ref::<Arc<T>>().cloned())
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