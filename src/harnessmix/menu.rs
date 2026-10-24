//! Plain terminal copy and choice parsing for the interactive chat menu.

/// An action selected while the menu is active.
///
/// The caller should call [`parse_menu_choice`] only when it is collecting a
/// menu choice; regular chat messages such as `1` remain ordinary messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    Login,
    NewSession,
    Config,
    Permissions,
    Exit,
    BackToChat,
    Cancel,
    InvalidChoice,
}

/// Prefixes for the two explicit terminal input modes.
pub const CHAT_PROMPT: &str = "Tú > ";
pub const MENU_PROMPT: &str = "Opción (0-5) > ";

/// Brief startup copy; detailed commands are available from the menu.
pub fn render_startup_banner() -> &'static str {
    "Antigravity · chat\nEscribe /menu para ver las opciones.\n"
}

/// Numbered actions stay within 60 columns and use no terminal escape codes.
pub fn render_menu() -> &'static str {
    "Opciones de conversación\n\
     1. Iniciar sesión con Google\n\
     2. Nueva sesión\n\
     3. Ver configuración\n\
     4. Permisos\n\
     5. Salir\n\
     0. Volver al chat\n\
     Escribe un número y pulsa Enter.\n\
     Modelo: /model   Esfuerzo: /effort   Uso: /usage\n"
}

/// Parses a choice only while the caller is explicitly in menu mode.
pub fn parse_menu_choice(input: &str) -> MenuAction {
    let choice = input.trim();
    if choice.is_empty()
        || choice == "\u{1b}"
        || choice.eq_ignore_ascii_case("q")
        || choice.eq_ignore_ascii_case("cancel")
    {
        return MenuAction::Cancel;
    }
    match choice {
        "1" => MenuAction::Login,
        "2" => MenuAction::NewSession,
        "3" => MenuAction::Config,
        "4" => MenuAction::Permissions,
        "5" => MenuAction::Exit,
        "0" => MenuAction::BackToChat,
        _ => MenuAction::InvalidChoice,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_menu_choice, render_menu, render_startup_banner, MenuAction, CHAT_PROMPT,
        MENU_PROMPT,
    };

    #[test]
    fn menu_is_numbered_and_fits_a_narrow_terminal() {
        let menu = render_menu();
        for label in [
            "1. Iniciar sesión con Google",
            "2. Nueva sesión",
            "3. Ver configuración",
            "4. Permisos",
            "5. Salir",
            "0. Volver al chat",
        ] {
            assert!(menu.contains(label));
        }
        assert!(menu.lines().all(|line| line.chars().count() <= 60));
        assert!(!menu.contains('\u{1b}'));
        assert!(CHAT_PROMPT.starts_with("Tú >"));
        assert!(MENU_PROMPT.contains("0-5"));
    }

    #[test]
    fn startup_banner_is_short_and_does_not_advertise_image_input() {
        let banner = render_startup_banner();
        assert!(banner.contains("/menu"));
        assert!(!banner.to_ascii_lowercase().contains("/image"));
        assert!(banner.lines().all(|line| line.chars().count() <= 60));
    }

    #[test]
    fn parses_each_menu_action() {
        assert_eq!(parse_menu_choice("1"), MenuAction::Login);
        assert_eq!(parse_menu_choice(" 2 "), MenuAction::NewSession);
        assert_eq!(parse_menu_choice("3"), MenuAction::Config);
        assert_eq!(parse_menu_choice("4"), MenuAction::Permissions);
        assert_eq!(parse_menu_choice("5"), MenuAction::Exit);
        assert_eq!(parse_menu_choice("0"), MenuAction::BackToChat);
    }

    #[test]
    fn invalid_and_cancelled_choices_are_distinct() {
        assert_eq!(parse_menu_choice("9"), MenuAction::InvalidChoice);
        assert_eq!(parse_menu_choice("hello"), MenuAction::InvalidChoice);
        assert_eq!(parse_menu_choice(""), MenuAction::Cancel);
        assert_eq!(parse_menu_choice("q"), MenuAction::Cancel);
        assert_eq!(parse_menu_choice("cancel"), MenuAction::Cancel);
        assert_eq!(parse_menu_choice("Cancel"), MenuAction::Cancel);
        assert_eq!(parse_menu_choice("\u{1b}"), MenuAction::Cancel);
    }
}
