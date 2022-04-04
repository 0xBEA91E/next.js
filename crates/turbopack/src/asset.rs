use turbo_tasks_fs::{FileContentVc, FileSystemPathVc};

use crate::reference::AssetReferencesSetVc;

#[turbo_tasks::value(shared)]
#[derive(Hash, PartialEq, Eq)]
pub struct AssetsSet {
    pub assets: Vec<AssetVc>,
}

#[turbo_tasks::value_impl]
impl AssetsSetVc {
    pub fn empty() -> Self {
        AssetsSet { assets: Vec::new() }.into()
    }
}

#[turbo_tasks::value_trait]
pub trait Asset {
    fn path(&self) -> FileSystemPathVc;
    fn content(&self) -> FileContentVc;
    fn references(&self) -> AssetReferencesSetVc;
}
