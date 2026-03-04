pub mod accessibility;
pub mod ai;
pub mod input;
pub mod screen;
pub mod types;

pub use accessibility::AccessibilityScanner;
pub use ai::DeepseekClient;
pub use ai::StreamChunk;
pub use input::ComputerController;
pub use screen::ScreenCapture;
pub use types::ControlCommand;
