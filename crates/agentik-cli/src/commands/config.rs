//! Configuration management commands.

use crate::ConfigAction;

pub async fn handle(action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Show => {
            println!("Current configuration:");
            // TODO: Load and display config
        }
        ConfigAction::Set { key, value } => {
            println!("Setting {} = {}", key, value);
            // TODO: Implement config set
        }
        ConfigAction::Edit => {
            println!("Opening config in editor...");
            // TODO: Open config in $EDITOR
        }
        ConfigAction::Reset => {
            println!("Resetting configuration to defaults...");
            // TODO: Implement config reset
        }
    }
    Ok(())
}
