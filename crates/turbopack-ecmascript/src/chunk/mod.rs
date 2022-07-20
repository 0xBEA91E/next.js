pub mod loader;

use std::fmt::Write;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbo_tasks::{primitives::StringVc, trace::TraceRawVcs, ValueToString, ValueToStringVc};
use turbo_tasks_fs::{File, FileContent, FileContentVc, FileSystemPathVc};
use turbopack_core::{
    asset::{Asset, AssetVc},
    chunk::{
        chunk_content, chunk_content_splitted, Chunk, ChunkContentResult, ChunkGroupReferenceVc,
        ChunkGroupVc, ChunkItemVc, ChunkReferenceVc, ChunkVc, ChunkableAssetVc, ChunkingContextVc,
        FromChunkableAsset, ModuleId, ModuleIdVc,
    },
    reference::{AssetReferenceVc, AssetReferencesVc},
};

use self::loader::ChunkGroupLoaderChunkItemVc;
use crate::{
    references::esm::EsmExportsVc,
    utils::{stringify_module_id, stringify_number, stringify_str, FormatIter},
};

#[turbo_tasks::value(Chunk, Asset, ValueToString)]
pub struct EcmascriptChunk {
    context: ChunkingContextVc,
    /// must implement [EcmascriptChunkPlaceable] too
    entry: AssetVc,
    evaluate: bool,
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkVc {
    #[turbo_tasks::function]
    pub fn new(context: ChunkingContextVc, entry: AssetVc) -> Self {
        Self::cell(EcmascriptChunk {
            context,
            entry,
            evaluate: false,
        })
    }
    #[turbo_tasks::function]
    pub fn new_evaluate(context: ChunkingContextVc, entry: AssetVc) -> Self {
        Self::cell(EcmascriptChunk {
            context,
            entry,
            evaluate: true,
        })
    }
}

#[turbo_tasks::function]
fn chunk_context(_context: ChunkingContextVc) -> EcmascriptChunkContextVc {
    EcmascriptChunkContextVc::cell(EcmascriptChunkContext {})
}

#[turbo_tasks::value]
pub struct EcmascriptChunkContentResult {
    pub chunk_items: Vec<EcmascriptChunkItemVc>,
    pub chunks: Vec<ChunkVc>,
    pub async_chunk_groups: Vec<ChunkGroupVc>,
    pub external_asset_references: Vec<AssetReferenceVc>,
}

impl From<ChunkContentResult<EcmascriptChunkItemVc>> for EcmascriptChunkContentResult {
    fn from(from: ChunkContentResult<EcmascriptChunkItemVc>) -> Self {
        EcmascriptChunkContentResult {
            chunk_items: from.chunk_items,
            chunks: from.chunks,
            async_chunk_groups: from.async_chunk_groups,
            external_asset_references: from.external_asset_references,
        }
    }
}

#[turbo_tasks::function]
async fn ecmascript_chunk_content(
    context: ChunkingContextVc,
    entry: AssetVc,
) -> Result<EcmascriptChunkContentResultVc> {
    let res = if let Some(res) = chunk_content::<EcmascriptChunkItemVc>(context, entry).await? {
        res
    } else {
        chunk_content_splitted::<EcmascriptChunkItemVc>(context, entry).await?
    };

    Ok(EcmascriptChunkContentResultVc::cell(res.into()))
}

#[turbo_tasks::value_impl]
impl Chunk for EcmascriptChunk {}

#[turbo_tasks::value_impl]
impl ValueToString for EcmascriptChunk {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell(format!(
            "chunk {}",
            self.entry.path().to_string().await?
        )))
    }
}

#[turbo_tasks::function]
async fn module_factory(content: EcmascriptChunkItemContentVc) -> Result<StringVc> {
    let content = content.await?;
    let mut args = vec![
        "r: __turbopack_require__",
        "i: __turbopack_import__",
        "s: __turbopack_esm__",
        "v: __turbopack_export_value__",
        "p: process",
    ];
    if content.options.module {
        args.push("m: module");
    }
    if content.options.exports {
        args.push("e: exports");
    }
    Ok(StringVc::cell(format!(
        "\n{}: (({{ {} }}) => (() => {{\n\n{}\n}})()),\n",
        match &*content.id.await? {
            ModuleId::Number(n) => stringify_number(*n),
            ModuleId::String(s) => stringify_str(s),
        },
        FormatIter(|| args.iter().copied().intersperse(", ")),
        content.inner_code
    )))
}

#[turbo_tasks::value_impl]
impl Asset for EcmascriptChunk {
    #[turbo_tasks::function]
    fn path(&self) -> FileSystemPathVc {
        self.context.as_chunk_path(self.entry.path(), ".js")
    }

    #[turbo_tasks::function]
    async fn content(self_vc: EcmascriptChunkVc) -> Result<FileContentVc> {
        let this = self_vc.await?;
        let content = ecmascript_chunk_content(this.context, this.entry);
        let c_context = chunk_context(this.context);
        let path = self_vc.path();
        let chunk_id = path.to_string();
        let contents = content
            .await?
            .chunk_items
            .iter()
            .map(|chunk_item| module_factory(chunk_item.content(c_context, this.context)))
            .collect::<Vec<_>>();
        let evaluate_chunks = if this.evaluate {
            Some(ChunkGroupVc::from_chunk(self_vc.into()).chunks())
        } else {
            None
        };
        let mut code = format!(
            "(self.TURBOPACK = self.TURBOPACK || []).push([{}, {{\n",
            stringify_str(&chunk_id.await?)
        );
        for module_factory in contents.iter() {
            code += &*module_factory.await?;
        }
        code += "\n}";
        if let Some(evaluate_chunks) = evaluate_chunks {
            let evaluate_chunks = evaluate_chunks.await?;
            let mut chunk_ids = Vec::new();
            for c in evaluate_chunks.iter() {
                if let Some(ecma_chunk) = EcmascriptChunkVc::resolve_from(c).await? {
                    if ecma_chunk != self_vc {
                        chunk_ids.push(stringify_str(&*c.path().to_string().await?));
                    }
                }
            }

            let condition = chunk_ids
                .into_iter()
                .map(|id| format!(" && chunks.has({})", id))
                .collect::<Vec<_>>()
                .join("");
            let module_id = c_context
                .id(EcmascriptChunkPlaceableVc::cast_from(this.entry))
                .await?;
            let entry_id = stringify_module_id(&module_id);
            let _ = write!(
                code,
                ", ({{ chunks, getModule }}) => {{
    if(!(true{condition})) return true;
    getModule(0, {entry_id})
}}"
            );
        }
        code += "]);\n";
        if this.evaluate {
            code += r#"(() => {
    if(Array.isArray(self.TURBOPACK)) {
        var array = self.TURBOPACK;
        var chunks = new Set();
        var runnable = [];
        var modules = {};
        var cache = {};
        let socket;
        // TODO: temporary solution
        var process = { env: { NODE_ENV: "development" } };
        var hOP = Object.prototype.hasOwnProperty;
        function require(from, id) {
            return getModule(from, id).exports;
        }
        var toStringTag = typeof Symbol !== "undefined" && Symbol.toStringTag;
        function esm(exports, getters) {
            Object.defineProperty(exports, "__esModule", { value: true });
            if(toStringTag) Object.defineProperty(exports, toStringTag, { value: "Module" });
            for(var key in getters) {
                if(hOP.call(getters, key)) {
                    Object.defineProperty(exports, key, { get: getters[key], enumerable: true, });
                }
            }
        }
        function exportValue(module, value) {
            module.exports = value;
        }
        function createGetter(obj, key) {
            return () => obj[key];
        }
        function interopEsm(raw, ns, allowExportDefault) {
            var getters = {};
            for(var key in raw) {
                getters[key] = createGetter(raw, key);
            }
            if(!(allowExportDefault && "default" in getters)) {
                getters["default"] = () => raw;
            }
            esm(ns, getters);
        }
        function importModule(from, id, allowExportDefault) {
            var module = getModule(from, id);
            var raw = module.exports;
            if(raw.__esModule) return raw;
            if(module.interopNamespace) return module.interopNamespace;
            var ns = module.interopNamespace = {};
            interopEsm(raw, ns, allowExportDefault);
            return ns;
        }
        function getModule(from, id) {
            if(hOP.call(cache, id)) {
                return cache[id];
            }
            var module = { exports: {}, loaded: false, id, parents: new Set(), children: new Set(), interopNamespace: undefined };
            cache[id] = module;
            var moduleFactory = modules[id];
            if(typeof moduleFactory != "function") {
                throw new Error(`Module ${id} was imported from module ${from}, but the module factory is not available`);
            }
            moduleFactory.call(module.exports, { e: module.exports, r: require.bind(null, id), i: importModule.bind(null, id), s: esm.bind(null, module.exports), v: exportValue.bind(null, module), m: module, c: cache, p: process });
            module.loaded = true;
            if(module.interopNamespace) {
                // in case of a circular dependency: cjs1 -> esm2 -> cjs1
                interopEsm(module.exports, module.interopNamespace);
            }
            return module;
        }
        var runtime = { chunks, modules, cache, getModule };
        function op([id, chunkModules, ...run]) {
            chunks.add(id);
            if(socket) socket.send(JSON.stringify(id));
            for(var m in chunkModules) {
                if(!modules[m]) modules[m] = chunkModules[m];
            }
            runnable.push(...run);
            runnable = runnable.filter(r => r(runtime))
        }
        self.TURBOPACK = { push: op };
        array.forEach(op);
        var connectingSocket = new WebSocket("ws" + location.origin.slice(4));
        connectingSocket.onopen = () => {
            socket = connectingSocket;
            for(var chunk of chunks) {
                socket.send(JSON.stringify(chunk));
            }
            socket.onmessage = (event) => {
                if(event.data === "refresh") location.reload();
            }
        }
    }
})();"#;
        }

        Ok(FileContent::Content(File::from_source(code)).into())
    }

    #[turbo_tasks::function]
    async fn references(&self) -> Result<AssetReferencesVc> {
        let content = ecmascript_chunk_content(self.context, self.entry).await?;
        let mut references = Vec::new();
        for r in content.external_asset_references.iter() {
            references.push(*r);
        }
        for chunk in content.chunks.iter() {
            references.push(ChunkReferenceVc::new_parallel(*chunk).into());
        }
        for chunk_group in content.async_chunk_groups.iter() {
            references.push(ChunkGroupReferenceVc::new(*chunk_group).into());
        }
        Ok(AssetReferencesVc::cell(references))
    }
}

#[turbo_tasks::value]
pub struct EcmascriptChunkContext {}

#[turbo_tasks::value_impl]
impl EcmascriptChunkContextVc {
    #[turbo_tasks::function]
    pub async fn id(self, placeable: EcmascriptChunkPlaceableVc) -> Result<ModuleIdVc> {
        Ok(ModuleId::String(placeable.to_string().await?.clone()).into())
    }

    #[turbo_tasks::function]
    pub async fn helper_id(self, name: &str, related_asset: Option<AssetVc>) -> Result<ModuleIdVc> {
        if let Some(related_asset) = related_asset {
            Ok(ModuleId::String(format!(
                "{}/__/{}",
                related_asset.path().to_string().await?,
                name
            ))
            .into())
        } else {
            Ok(ModuleId::String(name.to_string()).into())
        }
    }
}

#[turbo_tasks::value(shared)]
pub enum EcmascriptExports {
    EsmExports(EsmExportsVc),
    CommonJs,
    Value,
    None,
}

#[turbo_tasks::value_trait]
pub trait EcmascriptChunkPlaceable: Asset + ValueToString {
    fn as_chunk_item(&self, context: ChunkingContextVc) -> EcmascriptChunkItemVc;
    fn get_exports(&self) -> EcmascriptExportsVc;
}

#[turbo_tasks::value(shared)]
pub struct EcmascriptChunkItemContent {
    pub inner_code: String,
    pub id: ModuleIdVc,
    pub options: EcmascriptChunkItemOptions,
}

#[derive(PartialEq, Eq, Default, Clone, Serialize, Deserialize, TraceRawVcs)]
pub struct EcmascriptChunkItemOptions {
    pub module: bool,
    pub exports: bool,
}

#[turbo_tasks::value_trait]
pub trait EcmascriptChunkItem: ChunkItem {
    // TODO handle Source Maps, maybe via separate method "content_with_map"
    fn content(
        &self,
        chunk_context: EcmascriptChunkContextVc,
        context: ChunkingContextVc,
    ) -> EcmascriptChunkItemContentVc;
}

#[async_trait::async_trait]
impl FromChunkableAsset for EcmascriptChunkItemVc {
    async fn from_asset(context: ChunkingContextVc, asset: AssetVc) -> Result<Option<Self>> {
        if let Some(placeable) = EcmascriptChunkPlaceableVc::resolve_from(asset).await? {
            return Ok(Some(placeable.as_chunk_item(context)));
        }
        Ok(None)
    }

    async fn from_async_asset(
        _context: ChunkingContextVc,
        asset: ChunkableAssetVc,
    ) -> Result<Option<Self>> {
        Ok(Some(ChunkGroupLoaderChunkItemVc::new(asset).into()))
    }
}
