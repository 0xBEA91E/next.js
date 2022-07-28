use std::{io::stdin, sync::Arc};

use anyhow::Result;
use clap::Parser;
use regex::{NoExpand, Regex};
use swc::{
    common::{errors::ColorConfig, source_map::FileName, SourceMap},
    config::IsModule,
    ecmascript::{
        ast::EsVersion,
        parser::{Syntax, TsConfig},
    },
    try_with_handler, Compiler, HandlerOpts,
};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Whether to keep Span (location) markers
    #[clap(long, value_parser, default_value_t = false)]
    spans: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let mut contents = String::new();
    stdin().read_line(&mut contents)?;

    let sm = Arc::new(SourceMap::default());
    let file = sm.new_source_file(FileName::Anon, contents);
    let target = EsVersion::latest();
    let syntax = Syntax::Typescript(TsConfig {
        tsx: true,
        decorators: false,
        dts: false,
        no_early_errors: true,
    });

    let compiler = Compiler::new(sm.clone());
    let res = try_with_handler(
        sm,
        HandlerOpts {
            color: ColorConfig::Always,
            skip_filename: false,
        },
        |handler| compiler.parse_js(file, handler, target, syntax, IsModule::Unknown, None),
    );

    let print = format!("{:#?}", res?);

    let stripped = if args.spans {
        print
    } else {
        let span = Regex::new(r"(?m)^\s+span: Span \{[^}]*\},\n").unwrap();
        span.replace_all(&print, NoExpand("")).to_string()
    };

    let ws = Regex::new(r" {4}").unwrap();
    println!("{}", ws.replace_all(&stripped, NoExpand("  ")));

    Ok(())
}
