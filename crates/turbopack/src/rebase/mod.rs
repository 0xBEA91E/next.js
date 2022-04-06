use std::hash::Hash;

use anyhow::Result;
use turbo_tasks_fs::{FileContentVc, FileSystemPathVc};

use crate::{
    asset::{Asset, AssetVc},
    reference::{AssetReference, AssetReferenceVc, AssetReferencesSet, AssetReferencesSetVc},
    resolve::ResolveResultVc,
};

#[turbo_tasks::value(Asset)]
#[derive(Hash, PartialEq, Eq)]
pub struct RebasedAsset {
    source: AssetVc,
    input_dir: FileSystemPathVc,
    output_dir: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl RebasedAssetVc {
    pub fn new(source: AssetVc, input_dir: FileSystemPathVc, output_dir: FileSystemPathVc) -> Self {
        Self::slot(RebasedAsset {
            source: source,
            input_dir: input_dir,
            output_dir: output_dir,
        })
    }
}

#[turbo_tasks::value_impl]
impl Asset for RebasedAsset {
    async fn path(&self) -> FileSystemPathVc {
        FileSystemPathVc::rebase(
            self.source.path(),
            self.input_dir.clone(),
            self.output_dir.clone(),
        )
    }

    async fn content(&self) -> FileContentVc {
        self.source.content()
    }

    async fn references(&self) -> Result<AssetReferencesSetVc> {
        let input_references = self.source.references().await?;
        let mut references = Vec::new();
        for reference in input_references.references.iter() {
            references.push(
                RebasedAssetReference {
                    reference: reference.clone().resolve().await?,
                    input_dir: self.input_dir.clone(),
                    output_dir: self.output_dir.clone(),
                }
                .into(),
            );
        }
        Ok(AssetReferencesSet { references }.into())
    }
}

#[turbo_tasks::value(shared, AssetReference)]
#[derive(PartialEq, Eq)]
struct RebasedAssetReference {
    reference: AssetReferenceVc,
    input_dir: FileSystemPathVc,
    output_dir: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl AssetReference for RebasedAssetReference {
    async fn resolve_reference(&self) -> Result<ResolveResultVc> {
        let result = self.reference.resolve_reference().await?;
        Ok(result
            .map(
                |asset| {
                    let asset = RebasedAssetVc::new(
                        asset.clone(),
                        self.input_dir.clone(),
                        self.output_dir.clone(),
                    )
                    .into();
                    async { Ok(asset) }
                },
                |reference| {
                    let reference = RebasedAssetReference {
                        reference: reference.clone(),
                        input_dir: self.input_dir.clone(),
                        output_dir: self.output_dir.clone(),
                    }
                    .into();
                    async { Ok(reference) }
                },
            )
            .await?
            .into())
    }
}
