use zed::settings::LspSettings;
use zed_extension_api as zed;

struct PlantUmlExtension;

impl zed::Extension for PlantUmlExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        Ok(zed::Command {
            command: zed::node_binary_path()?,
            args: vec!["-e".to_string(), include_str!("lsp-stdio.cjs").to_string()],
            env: vec![(
                "PLANTUML_ZED_WORKTREE_ROOT".to_string(),
                worktree.root_path(),
            )],
        })
    }

    fn language_server_initialization_options(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree("plantuml-lsp", worktree)
            .ok()
            .and_then(|settings| settings.settings)
            .unwrap_or_else(|| zed::serde_json::json!({}));

        Ok(Some(zed::serde_json::json!({
            "worktreeRoot": worktree.root_path(),
            "settings": settings
        })))
    }
}

zed::register_extension!(PlantUmlExtension);
