use std::{
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};

/// `WithCleanup` can wrap any value
pub struct WithCleanup<T, F: FnOnce(T)> {
    // The order of declaration of members determines the drop order, going from top to bottom.
    // Due to the use of `ManuallyDrop` this is irrelevant here.
    inner: ManuallyDrop<T>,
    cleanup_fn: ManuallyDrop<F>,
}
impl<T, F: FnOnce(T)> WithCleanup<T, F> {
    pub fn new(inner: T, cleanup_fn: F) -> Self {
        Self {
            inner: ManuallyDrop::new(inner),
            cleanup_fn: ManuallyDrop::new(cleanup_fn),
        }
    }
    pub fn into_inner(mut self) -> (T, F) {
        unsafe {
            (
                ManuallyDrop::take(&mut self.inner),
                ManuallyDrop::take(&mut self.cleanup_fn),
            )
        }
    }
}
impl<T, F: FnOnce(T)> Drop for WithCleanup<T, F> {
    fn drop(&mut self) {
        let (inner, cleanup_fn) = unsafe {
            (
                ManuallyDrop::<T>::take(&mut self.inner),
                ManuallyDrop::<F>::take(&mut self.cleanup_fn),
            )
        };
        cleanup_fn(inner);
    }
}
impl<T, F: FnOnce(T)> Deref for WithCleanup<T, F> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T, F: FnOnce(T)> DerefMut for WithCleanup<T, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}