use std::collections::{HashSet, VecDeque};

use anyhow::Result;
use turbo_tasks::primitives::StringVc;

use crate::{
    asset::{AssetVc, AssetsVc},
    issue::IssueVc,
    resolve::{ResolveResult, ResolveResultVc},
};

pub mod source_map;

pub use source_map::SourceMapVc;

/// A reference to one or multiple [Asset]s or other special things.
/// There are a bunch of optional traits that can influence how these references
/// are handled. e. g. [ChunkableAssetReference], [AsyncLoadableReference] or
/// [ParallelChunkReference]
///
/// [Asset]: crate::asset::Asset
/// [ChunkableAssetReference]: crate::chunk::ChunkableAssetReference
/// [AsyncLoadableReference]: crate::chunk::AsyncLoadableReference
/// [ParallelChunkReference]: crate::chunk::ParallelChunkReference
#[turbo_tasks::value_trait]
pub trait AssetReference {
    fn resolve_reference(&self) -> ResolveResultVc;
    // TODO think about different types
    // fn kind(&self) -> AssetReferenceTypeVc;
    fn description(&self) -> StringVc;
}

/// Multiple [AssetReference]s
#[turbo_tasks::value(transparent)]
pub struct AssetReferences(Vec<AssetReferenceVc>);

#[turbo_tasks::value_impl]
impl AssetReferencesVc {
    /// An empty list of [AssetReference]s
    #[turbo_tasks::function]
    pub fn empty() -> Self {
        AssetReferencesVc::cell(Vec::new())
    }
}

/// Aggregates all [Asset]s referenced by an [Asset]. [AssetReference]
/// This does not include transitively references [Asset]s, but it includes
/// primary and secondary [Asset]s referenced.
///
/// [Asset]: crate::asset::Asset
#[turbo_tasks::function]
pub async fn all_referenced_assets(asset: AssetVc) -> Result<AssetsVc> {
    let references_set = asset.references().await?;
    let mut assets = Vec::new();
    let mut queue = VecDeque::new();
    for reference in references_set.iter() {
        queue.push_back(reference.resolve_reference());
    }
    // that would be non-deterministic:
    // while let Some(result) = race_pop(&mut queue).await {
    // match &*result? {
    while let Some(resolve_result) = queue.pop_front() {
        match &*resolve_result.await? {
            ResolveResult::Single(module, references) => {
                assets.push(*module);
                for reference in references {
                    queue.push_back(reference.resolve_reference());
                }
            }
            ResolveResult::Alternatives(modules, references) => {
                assets.extend(modules);
                for reference in references {
                    queue.push_back(reference.resolve_reference());
                }
            }
            ResolveResult::Special(_, references) => {
                for reference in references {
                    queue.push_back(reference.resolve_reference());
                }
            }
            ResolveResult::Keyed(_, _) => todo!(),
            ResolveResult::Unresolveable(references) => {
                for reference in references {
                    queue.push_back(reference.resolve_reference());
                }
            }
        }
    }
    Ok(AssetsVc::cell(assets))
}

/// Aggregates all [Asset]s referenced by an [Asset] including transitively
/// referenced [Asset]s. This basically gives all [Asset]s in a subgraph
/// starting from the passed [Asset].
#[turbo_tasks::function]
pub async fn all_assets(asset: AssetVc) -> Result<AssetsVc> {
    // TODO need to track import path here
    let mut queue = VecDeque::new();
    queue.push_back((asset, all_referenced_assets(asset)));
    let mut assets = HashSet::new();
    assets.insert(asset);
    while let Some((parent, references)) = queue.pop_front() {
        IssueVc::attach_context(
            parent.path(),
            "expanding references of asset".to_string(),
            references,
        )
        .await?;
        for asset in references.await?.iter() {
            if assets.insert(*asset) {
                queue.push_back((*asset, all_referenced_assets(*asset)));
            }
        }
    }
    Ok(AssetsVc::cell(assets.into_iter().collect()))
}
