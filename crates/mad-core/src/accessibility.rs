use accessibility::{AXAttribute, AXUIElement};
use active_win_pos_rs::get_active_window;
use anyhow::{Result, anyhow};
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::string::CFString;

pub struct AccessibilityScanner;

impl AccessibilityScanner {
    /// Captures the UI tree of the currently active window.
    /// Returns a structured string representation of the UI elements.
    /// If detailed is false, only returns the top-level structure.
    pub fn capture_active_window_tree(detailed: bool) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use dispatch::Queue;
            let mut result = Ok(String::new());

            Queue::main().exec_sync(|| {
                // 1. Get Active Window Info
                let active_window = match get_active_window() {
                    Ok(w) => w,
                    Err(_) => {
                        result = Err(anyhow!("Failed to get active window"));
                        return;
                    }
                };

                // 2. Create AXUIElement from PID
                let app_element = AXUIElement::application(active_window.process_id as i32);

                // 3. Find the focused window within the app
                let window_element = match app_element.attribute(&AXAttribute::focused_window()) {
                    Ok(w) => w,
                    Err(_) => {
                        // Fallback: try to find the window by title if focused_window fails
                        result = Ok(format!(
                            "Active App: {}\n(Could not access window UI tree - ensure Accessibility permissions are granted)",
                            active_window.app_name
                        ));
                        return;
                    }
                };

                // 4. Traverse the UI tree
                let mut tree_buffer = String::new();
                tree_buffer.push_str(&format!(
                    "App: {}\nWindow: {}\n",
                    active_window.app_name, active_window.title
                ));

                if detailed {
                    Self::traverse_element(&window_element, 0, &mut tree_buffer);
                } else {
                    tree_buffer
                        .push_str("(Detailed UI tree hidden. Use 'inspect' action to see details.)\n");
                    // Optionally show top-level elements like Menu Bar
                    if let Ok(children) = app_element.attribute(&AXAttribute::children()) {
                        for child in children.into_iter() {
                            let role = child
                                .attribute(&AXAttribute::role())
                                .map(|r| r.to_string())
                                .unwrap_or_default();
                            if role == "AXMenuBar" {
                                tree_buffer.push_str("[AXMenuBar]\n");
                            }
                        }
                    }
                }

                result = Ok(tree_buffer);
            });

            result
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(anyhow!("Capture tree not implemented for this platform"))
        }
    }

    /// Minimizes the currently active window.
    pub fn minimize_active_window() -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            use dispatch::Queue;
            let mut result = Ok(());

            Queue::main().exec_sync(|| {
                let active_window = match get_active_window() {
                    Ok(w) => w,
                    Err(_) => {
                        result = Err(anyhow!("Failed to get active window"));
                        return;
                    }
                };

                let app_element = AXUIElement::application(active_window.process_id as i32);

                if let Ok(window) = app_element.attribute(&AXAttribute::focused_window()) {
                    let minimized_attr = AXAttribute::new(&CFString::new("AXMinimized"));
                    if let Err(e) =
                        window.set_attribute(&minimized_attr, CFBoolean::true_value().as_CFType())
                    {
                        result = Err(anyhow!("Failed to minimize window: {:?}", e));
                    }
                } else {
                    result = Err(anyhow!("No focused window found"));
                }
            });

            result
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(anyhow!("Minimize not implemented for this platform"))
        }
    }

    /// Finds an element by matching text in title, description, or value.
    /// Returns the center coordinates (x, y).
    pub fn find_element_center(search_text: &str) -> Option<(i32, i32)> {
        #[cfg(target_os = "macos")]
        {
            use dispatch::Queue;
            let mut result = None;

            Queue::main().exec_sync(|| {
                let active_window = match get_active_window() {
                    Ok(w) => w,
                    Err(_) => {
                        tracing::error!("Failed to get active window for find_element_center");
                        return;
                    }
                };
                let app_element = AXUIElement::application(active_window.process_id as i32);

                // Check accessibility permissions
                if app_element.attribute(&AXAttribute::role()).is_err() {
                    tracing::error!(
                        "Accessibility permission missing or app not accessible (PID: {})",
                        active_window.process_id
                    );
                    return;
                }

                // 1. Try Focused Window first (Most likely target)
                if let Some(center) = app_element
                    .attribute(&AXAttribute::focused_window())
                    .ok()
                    .and_then(|w| Self::search_element(&w, search_text, 0))
                {
                    result = Some(center);
                    return;
                }

                // 2. Try Menu Bar (Find child with role "MenuBar")
                if let Ok(children) = app_element.attribute(&AXAttribute::children()) {
                    for child in children.into_iter() {
                        let role = child
                            .attribute(&AXAttribute::role())
                            .map(|r| r.to_string())
                            .unwrap_or_default();
                        if role == "AXMenuBar" {
                            let center = Self::search_element(&child, search_text, 0);
                            if center.is_some() {
                                result = center;
                                return;
                            }
                        }
                    }
                }

                // 3. Try All Windows (For secondary dialogs, sheets, or non-focused windows)
                if let Ok(windows) = app_element.attribute(&AXAttribute::windows()) {
                    for window in windows.into_iter() {
                        if let Some(center) = Self::search_element(&window, search_text, 0) {
                            result = Some(center);
                            return;
                        }
                    }
                }
            });

            result
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    fn search_element(element: &AXUIElement, text: &str, depth: usize) -> Option<(i32, i32)> {
        if depth > 20 {
            return None;
        }

        // 1. Search Children FIRST (Post-order traversal)
        // This ensures we find specific leaf nodes (Buttons, MenuItems) before their containers
        if let Ok(children) = element.attribute(&AXAttribute::children()) {
            for child in children.into_iter() {
                if let Some(center) = Self::search_element(&child, text, depth + 1) {
                    return Some(center);
                }
            }
        }

        // 2. Check Self
        let title = element
            .attribute(&AXAttribute::title())
            .map(|v| v.to_string())
            .unwrap_or_default();
        let description = element
            .attribute(&AXAttribute::description())
            .map(|v| v.to_string())
            .unwrap_or_default();
        let value_raw = element
            .attribute(&AXAttribute::value())
            .map(|v| format!("{:?}", v))
            .unwrap_or_default();
        let value = Self::clean_debug_value(&value_raw);
        let role = element
            .attribute(&AXAttribute::role())
            .map(|v| v.to_string())
            .unwrap_or("Unknown".to_string());

        // Skip container roles that shouldn't be clicked directly for item searches
        // unless there is an exact title match.
        // AXMenu center is usually near the top (e.g. System Settings), which explains the misclick.
        if (role == "AXMenu" || role == "AXMenuBar" || role == "AXApplication") && title != text {
            return None;
        }

        let search_lower = text.to_lowercase();

        // Strategy:
        // 1. If text is short (<= 2 chars), require exact match to avoid accidental hits.
        // 2. Do not match against 'value' if it's a TextField/TextArea (it's content, not label).
        let is_text_field = role == "TextField" || role == "TextArea" || role == "ComboBox";

        let match_found = if text.len() <= 2 {
            title.to_lowercase() == search_lower
                || description.to_lowercase() == search_lower
                || (!is_text_field && value.to_lowercase() == search_lower)
        } else {
            title.to_lowercase().contains(&search_lower)
                || description.to_lowercase().contains(&search_lower)
                || (!is_text_field && value.to_lowercase().contains(&search_lower))
        };

        if match_found {
            // Found match, get position
            let pos_attr = AXAttribute::new(&CFString::new("AXPosition"));
            let size_attr = AXAttribute::new(&CFString::new("AXSize"));

            if let (Ok(pos), Ok(size)) =
                (element.attribute(&pos_attr), element.attribute(&size_attr))
            {
                let pos_str = Self::clean_debug_value(&format!("{:?}", pos));
                let size_str = Self::clean_debug_value(&format!("{:?}", size));

                if let (Some((x, y)), Some((w, h))) =
                    (Self::parse_point(&pos_str), Self::parse_size(&size_str))
                {
                    tracing::info!(
                        "Found element '{}' (Role: {}, Title: '{}') at ({}, {}) size {}x{}",
                        text,
                        role,
                        title,
                        x,
                        y,
                        w,
                        h
                    );
                    return Some((x + w / 2, y + h / 2));
                }
            }
        }

        None
    }

    fn parse_point(s: &str) -> Option<(i32, i32)> {
        // Expected: "x=10.0, y=20.0"
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() < 2 {
            return None;
        }

        let x = parts[0].split('=').nth(1)?.trim().parse::<f64>().ok()? as i32;
        let y = parts[1].split('=').nth(1)?.trim().parse::<f64>().ok()? as i32;

        Some((x, y))
    }

    fn parse_size(s: &str) -> Option<(i32, i32)> {
        // Expected: "width=100.0, height=200.0" or "w=..., h=..."
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() < 2 {
            return None;
        }

        let w = parts[0].split('=').nth(1)?.trim().parse::<f64>().ok()? as i32;
        let h = parts[1].split('=').nth(1)?.trim().parse::<f64>().ok()? as i32;

        Some((w, h))
    }

    fn traverse_element(element: &AXUIElement, depth: usize, buffer: &mut String) {
        if depth > 20 {
            return;
        }

        let role_raw = element
            .attribute(&AXAttribute::role())
            .map(|v| v.to_string())
            .unwrap_or("Unknown".to_string());

        // Simplify Role Names
        let role = role_raw.trim_start_matches("AX");

        // Roles that are likely to have children we want to traverse
        let is_structural_container = [
            "Window",
            "Application",
            "ScrollArea",
            "SplitGroup",
            "Sheet",
            "Drawer",
            "Toolbar",
            "Table",
            "Outline",
            "List",
            "Browser",
            "TabGroup",
        ]
        .contains(&role);

        // Roles that are likely to be interactive or contain content
        let is_interactive = [
            "Button",
            "TextField",
            "TextArea",
            "Link",
            "CheckBox",
            "RadioButton",
            "MenuItem",
            "PopUpButton",
            "ComboBox",
            "Slider",
            "Tab",
            "Image",
        ]
        .contains(&role);

        // Roles that we should usually skip children traversal for (leaf nodes)
        let is_leaf =
            is_interactive || ["StaticText", "ValueIndicator", "LevelIndicator"].contains(&role);

        // Fetch other attributes only if potentially interesting
        let (title, description, value) =
            if is_interactive || is_structural_container || role == "StaticText" || depth < 3 {
                let title = element
                    .attribute(&AXAttribute::title())
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let description = element
                    .attribute(&AXAttribute::description())
                    .map(|v| v.to_string())
                    .unwrap_or_default();

                let value_raw = if is_interactive || role == "StaticText" || role == "ComboBox" {
                    element
                        .attribute(&AXAttribute::value())
                        .map(|v| format!("{:?}", v))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let value = Self::clean_debug_value(&value_raw);
                (title, description, value)
            } else {
                (String::new(), String::new(), String::new())
            };

        let has_content = !title.is_empty()
            || (!description.is_empty() && description != title)
            || !value.is_empty();

        // Logic:
        // 1. Always print interactive elements.
        // 2. Print containers only if they have a title or are structural at low depth.
        // 3. Print elements with content.
        let should_print = is_interactive
            || has_content
            || (is_structural_container && (!title.is_empty() || depth < 2));

        if should_print {
            let indent = "  ".repeat(depth);
            let mut info = format!("{}- [{}]", indent, role);

            if !title.is_empty() {
                info.push_str(&format!(" \"{}\"", title));
            }
            if !description.is_empty() && description != title {
                info.push_str(&format!(" ({})", description));
            }
            if !value.is_empty() && value != title {
                info.push_str(&format!(" value=\"{}\"", value));
            }

            // Always try to get position and size for AI to click accurately
            let pos_attr: AXAttribute<CFType> = AXAttribute::new(&CFString::new("AXPosition"));
            let size_attr: AXAttribute<CFType> = AXAttribute::new(&CFString::new("AXSize"));

            if let (Ok(pos), Ok(size)) =
                (element.attribute(&pos_attr), element.attribute(&size_attr))
            {
                let pos_str = Self::clean_debug_value(&format!("{:?}", pos));
                let size_str = Self::clean_debug_value(&format!("{:?}", size));

                if let (Some((x, y)), Some((w, h))) =
                    (Self::parse_point(&pos_str), Self::parse_size(&size_str))
                {
                    info.push_str(&format!(" @ [{},{},{},{}]", x, y, w, h));
                }
            }

            buffer.push_str(&info);
            buffer.push('\n');
        }

        // Traverse children only if NOT a leaf or if it's a structural container
        if (!is_leaf || is_structural_container || depth < 2)
            && let Ok(children) = element.attribute(&AXAttribute::children()) {
                let next_depth = depth + 1;
                for child in children.into_iter() {
                    Self::traverse_element(&child, next_depth, buffer);
                }
            }
    }

    /// cleans up debug output like "AXValue(CGPoint(x=10, y=20))" -> "(10, 20)"
    fn clean_debug_value(raw: &str) -> String {
        // Simple heuristic cleanup
        if raw.contains("CGPoint") {
            return raw
                .replace("AXValue", "")
                .replace("CGPoint", "")
                .replace("(", "")
                .replace(")", "")
                .trim()
                .to_string();
        }
        if raw.contains("CGSize") {
            return raw
                .replace("AXValue", "")
                .replace("CGSize", "")
                .replace("(", "")
                .replace(")", "")
                .trim()
                .to_string();
        }

        // Handle CoreFoundation types in debug output
        if raw.starts_with("CFString") {
            return raw
                .replace("CFString(", "")
                .trim_end_matches(')')
                .trim_matches('"')
                .to_string();
        }
        if raw.starts_with("CFBoolean") {
            return raw
                .replace("CFBoolean(", "")
                .trim_end_matches(')')
                .to_string();
        }
        if raw.starts_with("CFNumber") {
            // CFNumber(1) -> 1
            return raw
                .replace("CFNumber(", "")
                .trim_end_matches(')')
                .to_string();
        }

        // Handle cases like "Some(\"text\")" or "\"text\""
        raw.trim_matches('"').to_string()
    }
}
