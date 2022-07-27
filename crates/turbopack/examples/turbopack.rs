#![feature(trivial_bounds)]
#![feature(once_cell)]
#![feature(min_specialization)]

use std::{
    env::current_dir,
    fs,
    time::{Duration, Instant},
};

use anyhow::Result;
use tokio::{spawn, time::sleep};
use turbo_tasks::{util::FormatDuration, NothingVc, TurboTasks};
use turbo_tasks_fs::{DiskFileSystemVc, FileSystemPathVc, FileSystemVc};
use turbo_tasks_memory::{
    stats::Stats,
    viz::graph::{visualize_stats_tree, wrap_html},
    MemoryBackend,
};
use turbopack::{emit, rebase::RebasedAssetVc, register, GraphOptionsVc};
use turbopack_core::source_asset::SourceAssetVc;
use turbopack_ecmascript::target::CompileTargetVc;

#[tokio::main]
async fn main() -> Result<()> {
    register();

    let tt = TurboTasks::new(MemoryBackend::new());
    let start = Instant::now();

    let task = tt.spawn_root_task(|| {
        Box::pin(async {
            let root = current_dir().unwrap().to_str().unwrap().to_string();
            let disk_fs = DiskFileSystemVc::new("project".to_string(), root);
            disk_fs.await?.start_watching()?;

            // Smart Pointer cast
            let fs: FileSystemVc = disk_fs.into();
            let input = FileSystemPathVc::new(fs, "demo");
            let output = FileSystemPathVc::new(fs, "out");
            let entry = FileSystemPathVc::new(fs, "demo/index.js");

            let source = SourceAssetVc::new(entry);
            let context = turbopack::ModuleAssetContextVc::new(
                input,
                GraphOptionsVc::new(false, true, CompileTargetVc::current()),
            );
            let module = context.process(source.into());
            let rebased = RebasedAssetVc::new(module, input, output);
            emit(rebased.into());

            Ok(NothingVc::new().into())
        })
    });
    spawn({
        let tt = tt.clone();
        async move {
            tt.wait_task_completion(task, true).await.unwrap();
            println!("done in {}", FormatDuration(start.elapsed()));

            loop {
                let (elapsed, count) = tt.get_or_wait_update_info(Duration::from_millis(100)).await;
                println!("updated {} tasks in {}", count, FormatDuration(elapsed));
            }
        }
    })
    .await
    .unwrap();

    loop {
        println!("writing graph.html...");
        // create a graph
        let mut stats = Stats::new();

        let b = tt.backend();

        // graph root node
        stats.add_id(b, task);

        // graph tasks in cache
        b.with_all_cached_tasks(|task| {
            stats.add_id(b, task);
        });

        // prettify graph
        stats.merge_resolve();

        let tree = stats.treeify();

        // write HTML
        fs::write("graph.html", wrap_html(&visualize_stats_tree(tree))).unwrap();
        println!("graph.html written");

        sleep(Duration::from_secs(10)).await;
    }
}
