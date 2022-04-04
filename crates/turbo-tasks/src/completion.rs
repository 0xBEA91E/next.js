use crate::{self as turbo_tasks};

/// Just an empty type, but it's never equal to itself.
/// [CompletionVc] can be used as return value instead of `()`
/// to have a concrete reference that can be awaited.
/// It will invalidate the awaiting task everytime the referenced
/// task has been executed.
#[turbo_tasks::value(slot: new)]
pub struct Completion;

// no #[turbo_tasks::value_impl] to inline new into the caller task
// this ensures it's re-created on each execution
impl CompletionVc {
    pub fn new() -> Self {
        CompletionVc::slot(Completion)
    }
}
