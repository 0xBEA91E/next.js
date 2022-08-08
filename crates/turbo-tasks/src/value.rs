use std::{fmt::Debug, ops::Deref};

use crate::Typed;

/// Pass a value by value (`Value<Xxx>`) instead of by reference (`XxxVc`).
///
/// Persistent, requires serialization.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct Value<T: Typed> {
    inner: T,
}

impl<T: Typed> Value<T> {
    pub fn new(value: T) -> Self {
        Self { inner: value }
    }

    pub fn into_value(self) -> T {
        self.inner
    }
}

impl<T: Typed> Deref for Value<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Pass a value by value (`Value<Xxx>`) instead of by reference (`XxxVc`).
///
/// Doesn't require serialization, and won't be stored in the persistent cache
/// in the future.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct TransientValue<T> {
    inner: T,
}

impl<T> TransientValue<T> {
    pub fn new(value: T) -> Self {
        Self { inner: value }
    }

    pub fn into_value(self) -> T {
        self.inner
    }
}

impl<T> Deref for TransientValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
