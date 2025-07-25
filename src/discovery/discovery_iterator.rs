use crate::Result;
use async_trait::async_trait;

/// Async iterator trait for discovering scrobble edits incrementally
///
/// This trait is designed for iterators that yield individual results one at a time,
/// unlike AsyncPaginatedIterator which is designed for page-based iteration.
/// This is particularly useful for discovery operations that might make many API
/// requests and need to avoid rate limiting by yielding results incrementally.
#[async_trait(?Send)]
pub trait AsyncDiscoveryIterator<T> {
    /// Get the next item from the iterator
    ///
    /// Returns `Ok(Some(item))` if there's a next item available,
    /// `Ok(None)` if the iterator is exhausted, or `Err(e)` if an error occurred.
    async fn next(&mut self) -> Result<Option<T>>;

    /// Collect all remaining items from the iterator into a Vec
    ///
    /// This is a convenience method that calls `next()` repeatedly until
    /// the iterator is exhausted and collects all results.
    async fn collect_all(&mut self) -> Result<Vec<T>> {
        let mut items = Vec::new();
        while let Some(item) = self.next().await? {
            items.push(item);
        }
        Ok(items)
    }

    /// Take the first `n` items from the iterator
    ///
    /// This stops after collecting `n` items or when the iterator is exhausted,
    /// whichever comes first.
    async fn take(&mut self, n: usize) -> Result<Vec<T>> {
        let mut items = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(item) = self.next().await? {
                items.push(item);
            } else {
                break;
            }
        }
        Ok(items)
    }
}
