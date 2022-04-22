use std::{
    cell::RefCell,
    collections::HashSet,
    future::Future,
    hash::Hash,
    sync::{
        atomic::{AtomicU32, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use async_std::{
    task::{Builder, JoinHandle},
    task_local,
};
use crossbeam_epoch::Guard;
use event_listener::Event;
use flurry::HashMap as FHashMap;

use crate::{
    raw_vc::RawVc, task::NativeTaskFuture, task_input::TaskInput, trace::TraceRawVcs,
    NativeFunction, Task, TaskId, TraitType, Vc,
};

pub struct TurboTasks {
    next_task_id: AtomicU32,
    memory_tasks: FHashMap<TaskId, Task>,
    resolve_task_cache: FHashMap<(&'static NativeFunction, Vec<TaskInput>), TaskId>,
    native_task_cache: FHashMap<(&'static NativeFunction, Vec<TaskInput>), TaskId>,
    trait_task_cache: FHashMap<(&'static TraitType, String, Vec<TaskInput>), TaskId>,
    currently_scheduled_tasks: AtomicUsize,
    scheduled_tasks: AtomicUsize,
    start: Mutex<Option<Instant>>,
    last_update: Mutex<Option<(Duration, usize)>>,
    event: Event,
}

// TODO implement our own thread pool and make these thread locals instead
task_local! {
    /// The current TurboTasks instance
    static TURBO_TASKS: RefCell<Option<Arc<TurboTasks>>> = RefCell::new(None);

    /// Affected [Task]s, that are tracked during task execution
    /// These tasks will be invalidated when the execution finishes
    /// or before reading a slot value
    static TASKS_TO_NOTIFY: RefCell<Vec<TaskId>> = Default::default();
}

impl TurboTasks {
    // TODO better lifetime management for turbo tasks
    // consider using unsafe for the task_local turbo tasks
    // that should be safe as long tasks can't outlife turbo task
    // so we probably want to make sure that all tasks are joined
    // when trying to drop turbo tasks
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            next_task_id: AtomicU32::new(1),
            memory_tasks: FHashMap::new(),
            resolve_task_cache: FHashMap::new(),
            native_task_cache: FHashMap::new(),
            trait_task_cache: FHashMap::new(),
            currently_scheduled_tasks: AtomicUsize::new(0),
            scheduled_tasks: AtomicUsize::new(0),
            start: Default::default(),
            last_update: Default::default(),
            event: Event::new(),
        })
    }

    fn get_free_task_id(&self) -> TaskId {
        TaskId {
            id: self.next_task_id.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Creates a new root task
    pub fn spawn_root_task(
        self: &Arc<Self>,
        functor: impl Fn() -> NativeTaskFuture + Sync + Send + 'static,
    ) -> TaskId {
        let id = self.get_free_task_id();
        let task = Task::new_root(id, functor);
        self.memory_tasks.pin().insert(id, task);
        self.clone().schedule(id);
        id
    }

    // TODO make sure that all dependencies settle before reading them
    /// Creates a new root task, that is only executed once.
    /// Dependencies will not invalidate the task.
    pub fn spawn_once_task(
        self: &Arc<Self>,
        future: impl Future<Output = Result<RawVc>> + Send + 'static,
    ) -> TaskId {
        let id = self.get_free_task_id();
        let task = Task::new_once(id, future);
        self.memory_tasks.pin().insert(id, task);
        self.clone().schedule(id);
        id
    }

    pub async fn run_once<T: TraceRawVcs + Sync + Send + 'static>(
        self: &Arc<Self>,
        future: impl Future<Output = Result<T>> + Send + 'static,
    ) -> Result<T> {
        let task_id = self.spawn_once_task(async move {
            let result = future.await?;
            Ok(Vc::slot_new(Mutex::new(RefCell::new(Some(result)))).into())
        });
        let raw_result =
            Task::with_done_output(task_id, self, |_, output| output.read(task_id)).await?;
        // SAFETY: A Once task will never invalidate, therefore we don't need to track a
        // dependency
        let read_result =
            unsafe { raw_result.into_read_untracked::<Mutex<RefCell<Option<T>>>>(self.clone()) }
                .await?;
        let exchange = &*read_result;
        let guard = exchange.lock().unwrap();
        Ok(guard.take().unwrap())
    }

    /// Helper to get a [Task] from a HashMap or create a new one
    fn cached_call<K: Ord + PartialEq + Clone + Hash + Sync + Send + 'static>(
        self: &Arc<Self>,
        map: &FHashMap<K, TaskId>,
        key: K,
        create_new: impl FnOnce(TaskId) -> Task,
    ) -> RawVc {
        let map = map.pin();
        if let Some(task) = map.get(&key).map(|guard| *guard) {
            // fast pass without creating a new task
            Task::with_current(|parent, _| parent.connect_child(task, self));
            // TODO maybe force (background) scheduling to avoid inactive tasks hanging in
            // "in progress" until they become active
            RawVc::TaskOutput(task)
        } else {
            // slow pass with key lock
            let id = self.get_free_task_id();
            let new_task = create_new(id);
            let memory_tasks = self.memory_tasks.pin();
            memory_tasks.insert(id, new_task);
            let result_task = match map.try_insert(key, id) {
                Ok(_) => {
                    // This is the most likely case
                    id
                }
                Err(r) => {
                    memory_tasks.remove(&id);
                    // TODO give id back to the free list
                    *r.current
                }
            };
            Task::with_current(|parent, _| parent.connect_child(result_task, self));
            RawVc::TaskOutput(result_task)
        }
    }

    /// Call a native function with arguments.
    /// All inputs must be resolved.
    pub(crate) fn native_call(
        self: &Arc<Self>,
        func: &'static NativeFunction,
        inputs: Vec<TaskInput>,
    ) -> RawVc {
        debug_assert!(inputs.iter().all(|i| i.is_resolved() && !i.is_nothing()));
        self.cached_call(&self.native_task_cache, (func, inputs.clone()), |id| {
            Task::new_native(id, inputs, func)
        })
    }

    /// Calls a native function with arguments. Resolves arguments when needed
    /// with a wrapper [Task].
    pub fn dynamic_call(
        self: &Arc<Self>,
        func: &'static NativeFunction,
        inputs: Vec<TaskInput>,
    ) -> RawVc {
        if inputs.iter().all(|i| i.is_resolved() && !i.is_nothing()) {
            self.native_call(func, inputs)
        } else {
            self.cached_call(&self.resolve_task_cache, (func, inputs.clone()), |id| {
                Task::new_resolve_native(id, inputs, func)
            })
        }
    }

    /// Calls a trait method with arguments. First input is the `self` object.
    /// Uses a wrapper task to resolve
    pub fn trait_call(
        self: &Arc<Self>,
        trait_type: &'static TraitType,
        trait_fn_name: String,
        inputs: Vec<TaskInput>,
    ) -> RawVc {
        self.cached_call(
            &self.trait_task_cache,
            (trait_type, trait_fn_name.clone(), inputs.clone()),
            |id| Task::new_resolve_trait(id, trait_type, trait_fn_name, inputs),
        )
    }

    pub(crate) fn schedule(self: Arc<Self>, task_id: TaskId) -> JoinHandle<()> {
        if self
            .currently_scheduled_tasks
            .fetch_add(1, Ordering::AcqRel)
            == 0
        {
            *self.start.lock().unwrap() = Some(Instant::now());
        }
        self.scheduled_tasks.fetch_add(1, Ordering::AcqRel);
        Builder::new()
            // that's expensive
            // .name(format!("{:?} {:?}", &*task, &*task as *const Task))
            .spawn(async move {
                let execution = self.with_task_and_tt(task_id, |task| {
                    if task.execution_started(&self) {
                        self.with_task_and_tt(task_id, |task| Task::set_current(task, task_id));
                        let tt = self.clone();
                        TURBO_TASKS.with(|c| (*c.borrow_mut()) = Some(tt));
                        Some(task.execute(self.clone()))
                    } else {
                        None
                    }
                });
                if let Some(execution) = execution {
                    let result = execution.await;
                    self.with_task_and_tt(task_id, |task| {
                        if let Err(err) = &result {
                            println!("Task {} errored  {}", task, err);
                        }
                        task.execution_result(result);
                    });
                    self.notify_scheduled_tasks();
                    self.with_task_and_tt(task_id, |task| {
                        task.execution_completed(self.clone());
                    });
                }
                if self
                    .currently_scheduled_tasks
                    .fetch_sub(1, Ordering::AcqRel)
                    == 1
                {
                    // That's not super race-condition-safe, but it's only for statistical reasons
                    let total = self.scheduled_tasks.load(Ordering::Acquire);
                    self.scheduled_tasks.store(0, Ordering::Release);
                    if let Some(start) = *self.start.lock().unwrap() {
                        *self.last_update.lock().unwrap() = Some((start.elapsed(), total));
                    }
                    self.event.notify(usize::MAX);
                }
            })
            .unwrap()
    }

    pub async fn wait_done(self: &Arc<Self>) -> (Duration, usize) {
        self.event.listen().await;
        self.last_update.lock().unwrap().unwrap()
    }

    pub fn current() -> Option<Arc<Self>> {
        TURBO_TASKS.with(|c| (*c.borrow()).clone())
    }

    pub(crate) fn with_current<T>(func: impl FnOnce(&Arc<TurboTasks>) -> T) -> T {
        TURBO_TASKS.with(|c| {
            if let Some(arc) = c.borrow().as_ref() {
                func(arc)
            } else {
                panic!("Outside of TurboTasks");
            }
        })
    }

    pub(crate) fn with_task<T>(id: TaskId, func: impl FnOnce(&Task) -> T) -> T {
        Self::with_current(|tt| tt.with_task_and_tt(id, func))
    }

    pub(crate) fn with_task_and_tt<T>(&self, id: TaskId, func: impl FnOnce(&Task) -> T) -> T {
        func(&self.memory_tasks.pin().get(&id).unwrap())
    }

    pub(crate) fn schedule_background_job(
        self: Arc<Self>,
        job: impl Future<Output = ()> + Send + 'static,
    ) {
        Builder::new()
            .spawn(async move {
                TURBO_TASKS.with(|c| (*c.borrow_mut()) = Some(self.clone()));
                if self.currently_scheduled_tasks.load(Ordering::Acquire) != 0 {
                    let listener = self.event.listen();
                    if self.currently_scheduled_tasks.load(Ordering::Acquire) != 0 {
                        listener.await;
                    }
                }
                job.await;
            })
            .unwrap();
    }

    /// Eagerly notifies all tasks that were scheduled for notifications via
    /// `schedule_notify_tasks()`
    pub(crate) fn notify_scheduled_tasks(self: &Arc<TurboTasks>) {
        TASKS_TO_NOTIFY.with(|tasks| {
            for task in tasks.take().into_iter() {
                self.with_task_and_tt(task, |task| {
                    task.dependent_slot_updated(self);
                });
            }
        });
    }

    /// Enqueues tasks for notification of changed dependencies. This will
    /// eventually call `dependent_slot_updated()` on all tasks.
    pub(crate) fn schedule_notify_tasks<'a>(tasks_iter: impl Iterator<Item = &'a TaskId>) {
        TASKS_TO_NOTIFY.with(|tasks| {
            let mut list = tasks.borrow_mut();
            list.extend(tasks_iter);
        });
    }

    /// Schedules a background job that will deactive a list of tasks, when
    /// their active_parents count is still zero.
    pub(crate) fn schedule_deactivate_tasks(self: &Arc<Self>, tasks: Vec<TaskId>) {
        let tt = self.clone();
        self.clone().schedule_background_job(async move {
            Task::deactivate_tasks(tasks, tt);
        });
    }

    /// Schedules a background job that will decrease the active_parents count
    /// from each task by one and might deactive them after that.
    pub(crate) fn schedule_remove_tasks(self: &Arc<Self>, tasks: HashSet<TaskId>) {
        let tt = self.clone();
        self.clone().schedule_background_job(async move {
            Task::remove_tasks(tasks, tt);
        });
    }

    pub fn guard(&self) -> Guard {
        self.memory_tasks.guard()
    }

    /// Get a snapshot of all cached Tasks.
    pub fn cached_tasks_iter<'g>(
        &'g self,
        guard: &'g Guard,
    ) -> impl Iterator<Item = &'g Task> + 'g {
        self.memory_tasks.iter(guard).map(|(_, v)| v)
    }
}

/// see [TurboTasks] `dynamic_call`
pub fn dynamic_call(func: &'static NativeFunction, inputs: Vec<TaskInput>) -> RawVc {
    TurboTasks::with_current(|tt| tt.dynamic_call(func, inputs))
}

/// see [TurboTasks] `trait_call`
pub fn trait_call(
    trait_type: &'static TraitType,
    trait_fn_name: String,
    inputs: Vec<TaskInput>,
) -> RawVc {
    TurboTasks::with_current(|tt| tt.trait_call(trait_type, trait_fn_name, inputs))
}
