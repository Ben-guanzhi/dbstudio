use anyhow::anyhow;

use gpui::AssetSource;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../assets"]
#[include = "icons/**/*"]
#[exclude = "*.DS_Store"]
pub struct Assets;

pub struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(file) = Assets::get(path) {
            return Ok(Some(file.data));
        }
        if let Some(file) = gpui_kit_assets::Assets::get(path) {
            return Ok(Some(file.data));
        }
        Err(anyhow!("could not find asset at path \"{}\"", path))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        let mut paths: Vec<gpui::SharedString> = Assets::iter()
            .filter(|p| p.starts_with(path))
            .map(Into::into)
            .chain(
                gpui_kit_assets::Assets::iter()
                    .filter(|p| p.starts_with(path))
                    .map(Into::into),
            )
            .collect();
        paths.dedup();
        Ok(paths)
    }
}
