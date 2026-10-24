//! Native terminal adapter. Entry points compose injected ports; the domain and
//! application layers remain independent of terminal libraries.
#[path = "console/attachments.rs"]
mod attachments;
#[path = "console/cancellation.rs"]
mod cancellation;
#[path = "console/commands.rs"]
mod commands;
#[path = "console/configuration.rs"]
mod configuration;
#[path = "console/context.rs"]
mod context;
#[path = "console/entry.rs"]
mod entry;
#[path = "console/events.rs"]
mod events;
#[path = "console/live_commands.rs"]
mod live_commands;
#[path = "console/operations.rs"]
mod operations;
#[path = "console/output.rs"]
mod output;
#[path = "console/permissions.rs"]
mod permissions;
#[path = "console/ports.rs"]
mod ports;
#[path = "console/runtime.rs"]
mod runtime;
#[path = "console/session.rs"]
mod session;
#[path = "console/settings.rs"]
mod settings;
#[path = "console/state.rs"]
mod state;
#[path = "console/terminal.rs"]
mod terminal;
#[path = "console/view.rs"]
mod view;

pub use entry::{login, run_with_options};
pub(crate) use ports::ClipboardContent;
pub use ports::ConsoleResult;

use crate::{activity::Activity, menu};
use harness_domain::conversation::{ConversationEvent, PermissionPolicy};

use attachments::{enqueue_image, finish_clipboard, finish_image, image_path, request_clipboard};
use cancellation::{begin_cancel, start_cancel, wait_until};
use commands::{is_setting_command, process_idle_line};
use events::{ConsoleObserver, UiEvent, permission_action};
use live_commands::handle_live_line;
use operations::{spawn_operation, wait_setup};
use permissions::dispatch_event;
use ports::{ConsoleInput, ConsoleOutput, InputEvent};
use runtime::run_loop;
use session::{choose_auth_method, open_session};
use state::{
    UiChannels, UiState, interrupt_exits_when_armed, queue_prompt, ready_prompt, take_images,
};
use view::{current_prompt, render_activity};

#[derive(Clone, Copy, Debug)]
pub struct ConsoleOptions {
    pub permissions: PermissionPolicy,
    pub animate: bool,
}

impl Default for ConsoleOptions {
    fn default() -> Self {
        Self {
            permissions: PermissionPolicy::Ask,
            animate: true,
        }
    }
}

#[cfg(test)]
#[path = "console/tests.rs"]
mod tests;
