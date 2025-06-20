//! Convenient macros for Hub integration
//!
//! This module provides macros that make it easy to add Hub support to existing CLI tools.

/// Macro to create a progress bar if Hub is available, otherwise do nothing
#[macro_export]
macro_rules! hub_progress {
    ($current:expr, $total:expr, $message:expr) => {{
        if let Ok(Some(mut client)) = crate::create_hub_client().await {
            let _ = client.show_progress($current, $total, $message.to_string()).await;
        }
    }};
}

/// Macro to create a table if Hub is available, otherwise print to stdout
#[macro_export]
macro_rules! hub_table {
    ($headers:expr, $data:expr) => {{
        if let Ok(Some(mut client)) = crate::create_hub_client().await {
            let table = crate::table_from_data($headers, $data);
            let _ = client.show_ui_component(
                hub_protocol::messages::UiMessagePayload::Table(table)
            ).await;
        } else {
            // Fallback to CLI table
            println!("{}", $headers.join("\t"));
            for row in $data {
                let cells: Vec<String> = $headers.iter()
                    .map(|h| row.get(h).cloned().unwrap_or_default())
                    .collect();
                println!("{}", cells.join("\t"));
            }
        }
    }};
}

/// Macro to conditionally execute code only when Hub is available
#[macro_export]
macro_rules! if_hub {
    ($code:block) => {{
        if crate::is_hub_mode() {
            $code
        }
    }};
}

/// Macro to execute different code for Hub vs CLI mode
#[macro_export]
macro_rules! hub_or_cli {
    (hub: $hub_code:block, cli: $cli_code:block) => {{
        if crate::is_hub_mode() {
            $hub_code
        } else {
            $cli_code
        }
    }};
}

/// Re-export macros so they can be used with `use hub_sdk::*;`
pub use hub_progress;
pub use hub_table;
pub use if_hub;
pub use hub_or_cli;