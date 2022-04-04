use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_std::task::block_on;
use criterion::{black_box, Criterion};
use swc_common::{FilePathMapping, Mark, SourceMap, GLOBALS};
use swc_ecma_transforms_base::resolver::resolver_with_mark;
use swc_ecmascript::{ast::EsVersion, parser::parse_file_as_module, visit::VisitMutWith};
use turbopack::{
    __internals::test_utils,
    analyzer::{
        graph::{create_graph, EvalContext},
        linker::{link, LinkCache},
        test_utils::visitor,
    },
};

pub fn benchmark(c: &mut Criterion) {
    let mut tests_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    tests_dir.push("tests");
    tests_dir.push("analyzer");
    tests_dir.push("graph");
    let results = fs::read_dir(tests_dir).unwrap();
    for result in results {
        let result = result.unwrap();
        if result.file_type().unwrap().is_dir() {
            let name = result.file_name();
            let name = name.to_string_lossy();
            let mut input = result.path();
            input.push("input.js");

            let cm = Arc::new(SourceMap::new(FilePathMapping::empty()));
            let fm = cm.load_file(&input).unwrap();
            GLOBALS.set(&swc_common::Globals::new(), || {
                let mut m = parse_file_as_module(
                    &fm,
                    Default::default(),
                    EsVersion::latest(),
                    None,
                    &mut vec![],
                )
                .unwrap();

                let top_level_mark = Mark::fresh(Mark::root());
                m.visit_mut_with(&mut resolver_with_mark(top_level_mark));

                let eval_context = EvalContext::new(&m, top_level_mark);

                let var_graph = create_graph(&m, &eval_context);

                let mut group = c.benchmark_group(name.as_ref());
                group.bench_function("create_graph", move |b| {
                    b.iter(|| create_graph(&m, &eval_context));
                });
                group.bench_function("link", move |b| {
                    b.iter(|| {
                        let cache = Mutex::new(LinkCache::new());
                        for val in var_graph.values.values() {
                            block_on(link(
                                &var_graph,
                                val.clone(),
                                &(|val| Box::pin(visitor(val))),
                                &cache,
                            ))
                            .unwrap();
                        }
                    });
                });
            });
        }
    }
}
