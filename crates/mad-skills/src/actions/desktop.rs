use mad_core::ControlCommand;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DesktopAction {
    Click(ClickArgs),
    ClickComponent(ClickComponentArgs),
    Type(TypeArgs),
    Key(KeyArgs),
    Wait(WaitArgs),
    Minimize(MinimizeArgs),
    Inspect(InspectArgs),
    Screenshot(ScreenshotArgs),
}

impl DesktopAction {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        match self {
            DesktopAction::Click(args) => args.to_commands(),
            DesktopAction::ClickComponent(args) => args.to_commands(),
            DesktopAction::Type(args) => args.to_commands(),
            DesktopAction::Key(args) => args.to_commands(),
            DesktopAction::Wait(args) => args.to_commands(),
            DesktopAction::Minimize(args) => args.to_commands(),
            DesktopAction::Inspect(args) => args.to_commands(),
            DesktopAction::Screenshot(args) => args.to_commands(),
        }
    }

    pub fn description() -> &'static str {
        r#"
AVAILABLE ACTIONS:
1. { "action": "key", "keys": ["Modifier", "Key"] }
   - Press a key combination.
   - Modifiers: "Command", "Control", "Option", "Shift".
   - Keys: "Enter", "Space", "Tab", "Escape", "Backspace", "Up", "Down", "Left", "Right", "PageUp", "PageDown", "Home", "End", "F1" to "F12", or single characters like "n", "t".
   - Example: { "action": "key", "keys": ["Command", "Space"] }

2. { "action": "type", "text": "string" }
   - Type a string of text.
   - Example: { "action": "type", "text": "https://www.bing.com" }

3. { "action": "click", "x": integer, "y": integer }
   - Click at specific coordinates.
   - Example: { "action": "click", "x": 500, "y": 300 }

4. { "action": "click_component", "text": "string" }
   - Find and click a UI element by its text (title, description, or value).
   - IMPORTANT: Avoid using this with very short or dynamic values (like single characters or current text in a field). Prefer stable titles or coordinates.
   - Example: { "action": "click_component", "text": "Save" }

5. { "action": "wait", "seconds": integer }
    - Wait for UI to update.
    - REQUIRED after opening apps or loading pages.
    - Example: { "action": "wait", "seconds": 3 }

 6. { "action": "minimize" }
    - Minimize the current control window to access the desktop.
    - Example: { "action": "minimize" }

 7. { "action": "inspect" }
    - Get a detailed UI tree of the active window.
    - Use this when you need to find specific elements.
    - Example: { "action": "inspect" }

 8. { "action": "screenshot" }
    - Take a screenshot of the current screen.
    - Use this when you need to "see" the actual UI state for confirmation.
    - Example: { "action": "screenshot" }
"#
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {}

impl ScreenshotArgs {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        vec![ControlCommand::Screenshot]
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct InspectArgs {}

impl InspectArgs {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        vec![ControlCommand::Inspect]
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClickArgs {
    /// The x coordinate to click
    pub x: i32,
    /// The y coordinate to click
    pub y: i32,
}

impl ClickArgs {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        vec![
            ControlCommand::MoveMouse(self.x, self.y),
            ControlCommand::Click,
        ]
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClickComponentArgs {
    /// The text to find and click
    pub text: String,
}

impl ClickComponentArgs {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        vec![ControlCommand::ClickComponent(self.text)]
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TypeArgs {
    /// The text to type
    pub text: String,
}

impl TypeArgs {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        let text = self.text;
        // If text is long or URL, use Paste, else Type
        if text.len() > 50 || text.starts_with("http") {
            vec![ControlCommand::Paste(text)]
        } else {
            vec![ControlCommand::Type(text)]
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct KeyArgs {
    /// The keys to press (e.g., ["Command", "Space"])
    pub keys: Vec<String>,
}

impl KeyArgs {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        vec![ControlCommand::KeySequence(self.keys)]
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WaitArgs {
    /// The number of seconds to wait
    pub seconds: u64,
}

impl WaitArgs {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        vec![ControlCommand::Wait(self.seconds)]
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MinimizeArgs;

impl MinimizeArgs {
    pub fn to_commands(self) -> Vec<ControlCommand> {
        vec![ControlCommand::Minimize]
    }
}
