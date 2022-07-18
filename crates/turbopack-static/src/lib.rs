#![feature(min_specialization)]

use anyhow::{anyhow, Result};
use turbo_tasks::{primitives::StringVc, ValueToString, ValueToStringVc};
use turbo_tasks_fs::{FileContent, FileContentVc, FileSystemPathVc};
use turbopack_core::{
    asset::{Asset, AssetVc},
    chunk::{ChunkItem, ChunkItemVc, ChunkVc, ChunkableAsset, ChunkableAssetVc, ChunkingContextVc},
    context::AssetContextVc,
    reference::{AssetReference, AssetReferenceVc, AssetReferencesVc},
    resolve::{ResolveResult, ResolveResultVc},
};
use turbopack_css::embed::{CssEmbed, CssEmbedVc, CssEmbeddable, CssEmbeddableVc};
use turbopack_ecmascript::{
    chunk::{
        EcmascriptChunkContextVc, EcmascriptChunkItem, EcmascriptChunkItemContent,
        EcmascriptChunkItemContentVc, EcmascriptChunkItemOptions, EcmascriptChunkItemVc,
        EcmascriptChunkPlaceable, EcmascriptChunkPlaceableVc, EcmascriptChunkVc,
    },
    utils::stringify_str,
};

#[turbo_tasks::value(
    Asset,
    EcmascriptChunkPlaceable,
    ChunkableAsset,
    CssEmbeddable,
    ValueToString
)]
#[derive(Clone)]
pub struct ModuleAsset {
    pub source: AssetVc,
    pub context: AssetContextVc,
}

#[turbo_tasks::value_impl]
impl ModuleAssetVc {
    #[turbo_tasks::function]
    pub fn new(source: AssetVc, context: AssetContextVc) -> Self {
        Self::cell(ModuleAsset { source, context })
    }

    #[turbo_tasks::function]
    async fn static_asset(
        self_vc: ModuleAssetVc,
        context: ChunkingContextVc,
    ) -> Result<StaticAssetVc> {
        Ok(StaticAssetVc::cell(StaticAsset {
            context,
            source: self_vc.await?.source,
        }))
    }
}

#[turbo_tasks::value_impl]
impl Asset for ModuleAsset {
    #[turbo_tasks::function]
    fn path(&self) -> FileSystemPathVc {
        self.source.path()
    }
    #[turbo_tasks::function]
    fn content(&self) -> FileContentVc {
        self.source.content()
    }
    #[turbo_tasks::function]
    async fn references(&self) -> Result<AssetReferencesVc> {
        Ok(AssetReferencesVc::empty())
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAsset for ModuleAsset {
    #[turbo_tasks::function]
    fn as_chunk(self_vc: ModuleAssetVc, context: ChunkingContextVc) -> ChunkVc {
        EcmascriptChunkVc::new(context, self_vc.into()).into()
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for ModuleAsset {
    #[turbo_tasks::function]
    fn as_chunk_item(self_vc: ModuleAssetVc, context: ChunkingContextVc) -> EcmascriptChunkItemVc {
        ModuleChunkItemVc::cell(ModuleChunkItem {
            module: self_vc,
            context,
            static_asset: self_vc.static_asset(context),
        })
        .into()
    }
}

#[turbo_tasks::value_impl]
impl CssEmbeddable for ModuleAsset {
    #[turbo_tasks::function]
    fn as_css_embed(self_vc: ModuleAssetVc, context: ChunkingContextVc) -> CssEmbedVc {
        StaticCssEmbedVc::cell(StaticCssEmbed {
            static_asset: self_vc.static_asset(context),
        })
        .into()
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for ModuleAsset {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell(format!(
            "{} (static)",
            self.source.path().to_string().await?
        )))
    }
}

#[turbo_tasks::value(Asset)]
struct StaticAsset {
    context: ChunkingContextVc,
    source: AssetVc,
}

#[turbo_tasks::value_impl]
impl Asset for StaticAsset {
    #[turbo_tasks::function]
    async fn path(&self) -> Result<FileSystemPathVc> {
        let source_path = self.source.path();
        let content = self.source.content();
        let content_hash = turbopack_hash::hash_md4(match *content.await? {
            FileContent::Content(ref file) => file.content(),
            _ => return Err(anyhow!("StaticAsset::path: unsupported file content")),
        });
        let content_hash_b16 = turbopack_hash::encode_base16(&content_hash);
        let asset_path = match source_path.await?.extension() {
            Some(ext) => format!("{hash}.{ext}", hash = content_hash_b16, ext = ext),
            None => content_hash_b16,
        };
        Ok(self.context.asset_path(&asset_path))
    }

    #[turbo_tasks::function]
    fn content(&self) -> FileContentVc {
        self.source.content()
    }

    #[turbo_tasks::function]
    fn references(&self) -> AssetReferencesVc {
        AssetReferencesVc::empty()
    }
}

#[turbo_tasks::value(AssetReference)]
struct StaticAssetReference {
    static_asset: StaticAssetVc,
}

#[turbo_tasks::value_impl]
impl AssetReference for StaticAssetReference {
    #[turbo_tasks::function]
    async fn resolve_reference(&self) -> Result<ResolveResultVc> {
        Ok(ResolveResult::Single(self.static_asset.into(), Vec::new()).into())
    }

    #[turbo_tasks::function]
    async fn description(&self) -> Result<StringVc> {
        Ok(StringVc::cell(format!(
            "static(url) {}",
            self.static_asset.path().await?,
        )))
    }
}

#[turbo_tasks::value(ChunkItem, EcmascriptChunkItem)]
struct ModuleChunkItem {
    module: ModuleAssetVc,
    context: ChunkingContextVc,
    static_asset: StaticAssetVc,
}

#[turbo_tasks::value_impl]
impl ChunkItem for ModuleChunkItem {
    #[turbo_tasks::function]
    fn references(&self) -> AssetReferencesVc {
        AssetReferencesVc::cell(vec![StaticAssetReferenceVc::cell(StaticAssetReference {
            static_asset: self.static_asset,
        })
        .into()])
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for ModuleChunkItem {
    #[turbo_tasks::function]
    async fn content(
        &self,
        chunk_context: EcmascriptChunkContextVc,
        _context: ChunkingContextVc,
    ) -> Result<EcmascriptChunkItemContentVc> {
        Ok(EcmascriptChunkItemContent {
            inner_code: format!(
                "__turbopack_export_value__({path});",
                path = stringify_str(&format!("/{}", &*self.static_asset.path().await?))
            ),
            id: chunk_context.id(EcmascriptChunkPlaceableVc::cast_from(self.module)),
            options: EcmascriptChunkItemOptions {
                ..Default::default()
            },
        }
        .into())
    }
}

#[turbo_tasks::value(CssEmbed)]
struct StaticCssEmbed {
    static_asset: StaticAssetVc,
}

#[turbo_tasks::value_impl]
impl CssEmbed for StaticCssEmbed {
    #[turbo_tasks::function]
    fn references(&self) -> AssetReferencesVc {
        AssetReferencesVc::cell(vec![StaticAssetReferenceVc::cell(StaticAssetReference {
            static_asset: self.static_asset,
        })
        .into()])
    }
}

pub fn register() {
    turbo_tasks::register();
    turbo_tasks_fs::register();
    turbopack_core::register();
    turbopack_ecmascript::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}
