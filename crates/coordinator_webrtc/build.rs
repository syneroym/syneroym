//! Build script for `syneroym-coordinator-webrtc`.
//!
//! This script is responsible for preparing JavaScript assets (`sw.js` and
//! `peer-proxy.js`). In development mode, it simply copies the assets. In
//! release mode, it aggressively parses and minifies the JavaScript using
//! `swc_core` to reduce payload sizes, applying dead-code elimination,
//! tree-shaking, and top-level mangling.

#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::{env, fs, path::Path};

use swc_core::{
    common::{FileName, GLOBALS, Globals, Mark, SourceMap, sync::Lrc},
    ecma::{
        ast::Program,
        codegen::{Config as CodegenConfig, Emitter, text_writer::JsWriter},
        minifier::{
            self,
            option::{
                CompressOptions, ExtraOptions, MangleOptions, MinifyOptions, TopLevelOptions,
            },
        },
        parser::{Parser, StringInput, Syntax},
        transforms::base,
        visit::VisitMutWith,
    },
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = env::var("OUT_DIR")?;
    let profile = env::var("PROFILE")?;
    let templates = Path::new("templates");

    for file in &["sw.js", "peer-proxy.js"] {
        let src = templates.join(file);
        println!("cargo::rerun-if-changed={}", src.display());

        let source = fs::read_to_string(&src)?;
        let output = if profile == "release" {
            GLOBALS.set(&Globals::new(), || {
                let cm = Lrc::new(SourceMap::default());
                let fm = cm.new_source_file(FileName::Anon.into(), source.clone());
                let mut parser =
                    Parser::new(Syntax::Es(Default::default()), StringInput::from(&*fm), None);
                let module = parser.parse_module().expect("Failed to parse JS");

                let unresolved_mark = Mark::new();
                let top_level_mark = Mark::new();

                let mut program = Program::Module(module);
                program.visit_mut_with(&mut base::resolver(unresolved_mark, top_level_mark, false));

                let minify_extra_options =
                    ExtraOptions { unresolved_mark, top_level_mark, mangle_name_cache: None };

                let compress_opts = CompressOptions {
                    dead_code: true,
                    unused: true,
                    top_level: Some(TopLevelOptions { functions: true }),
                    ..Default::default()
                };

                let mangle_opts = MangleOptions { top_level: Some(true), ..Default::default() };

                let program = minifier::optimize(
                    program,
                    cm.clone(),
                    None,
                    None,
                    &MinifyOptions {
                        compress: Some(compress_opts),
                        mangle: Some(mangle_opts),
                        ..Default::default()
                    },
                    &minify_extra_options,
                );

                let mut buf = vec![];
                {
                    let mut emitter = Emitter {
                        cfg: CodegenConfig::default().with_minify(true),
                        cm: cm.clone(),
                        comments: None,
                        wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
                    };
                    emitter.emit_program(&program).unwrap();
                }
                String::from_utf8(buf).unwrap()
            })
        } else {
            source
        };

        fs::write(Path::new(&out_dir).join(file), output)?;
    }

    Ok(())
}
