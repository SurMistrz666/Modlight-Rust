//! The menu bar icon and its menu (`TrayIconService`, `MainMenuBuilder.BuildTrayMenu`).
//!
//! Only in the real app: QA runs never create it. The menu carries what exists after
//! phase 1, in upstream's order: the account (sign in, or "Signed in as" with its
//! submenu), Settings, Show Modlight, Exit. Capture, upload and the task menus join it
//! with the phases that build them.

use std::sync::mpsc::{Receiver, channel};

use modlight_core::l10n::{Arg, format, s, t};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::commands::Command;
use crate::state::AppState;

const SIGN_IN: &str = "sign-in";
const ACCOUNT_SETTINGS: &str = "account-settings";
const MODLIGHT_ACCOUNT: &str = "modlight-account";
const SIGN_OUT: &str = "sign-out";
const SETTINGS: &str = "settings";
const SHOW: &str = "show";
const EXIT: &str = "exit";

pub struct Tray {
    icon: TrayIcon,
    /// The account line the menu was built for; a change rebuilds it.
    built_for: String,
    events: Receiver<String>,
}

fn build_menu(state: &AppState) -> Menu {
    let menu = Menu::new();
    let result = (|| -> Result<(), tray_icon::menu::Error> {
        if state.account.is_signed_in() {
            let label = format(
                t(s::MainMenuBuilder_SignedInAs),
                &[Arg::from(state.account_label())],
            );
            let account = Submenu::new(label, true);
            account.append(&MenuItem::with_id(
                ACCOUNT_SETTINGS,
                t(s::MainMenuBuilder_AccountSettings),
                true,
                None,
            ))?;
            account.append(&MenuItem::with_id(
                MODLIGHT_ACCOUNT,
                t(s::MainMenuBuilder_ModlightAccount),
                true,
                None,
            ))?;
            account.append(&PredefinedMenuItem::separator())?;
            account.append(&MenuItem::with_id(
                SIGN_OUT,
                t(s::MainMenuBuilder_SignOut),
                true,
                None,
            ))?;
            menu.append(&account)?;
        } else {
            menu.append(&MenuItem::with_id(
                SIGN_IN,
                t(s::MainMenuBuilder_SignIn),
                true,
                None,
            ))?;
        }
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&MenuItem::with_id(
            SETTINGS,
            t(s::SettingsWindow_WindowTitle),
            true,
            None,
        ))?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&MenuItem::with_id(
            SHOW,
            t(s::MainMenuBuilder_ShowShareX),
            true,
            None,
        ))?;
        menu.append(&MenuItem::with_id(
            EXIT,
            t(s::MainMenuBuilder_Exit),
            true,
            None,
        ))?;
        Ok(())
    })();
    if let Err(error) = result {
        log::warn!("Modlight: the tray menu could not be built: {error}");
    }
    menu
}

fn signature(state: &AppState) -> String {
    format!(
        "{}|{}|{:?}",
        state.account.is_signed_in(),
        state.account_label(),
        state.language
    )
}

impl Tray {
    pub fn new(ctx: &egui::Context, state: &AppState) -> Option<Tray> {
        let icon = crate::app::app_icon().and_then(|image| {
            let small =
                image::imageops::resize(&image, 36, 36, image::imageops::FilterType::Lanczos3);
            Icon::from_rgba(small.into_raw(), 36, 36).ok()
        })?;
        let (sender, events) = channel();
        let wake = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let id = event.id().0.clone();
            if id == EXIT {
                // The window may be hidden and not drawing: leave from here. The settings
                // are saved as they change, so there is nothing left to write.
                std::process::exit(0);
            }
            if id != SIGN_OUT {
                modlight_macos::system::show_app_windows();
            }
            let _ = sender.send(id);
            wake.request_repaint();
        }));
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(modlight_core::APP_NAME)
            .with_menu(Box::new(build_menu(state)))
            .build()
            .map_err(|error| log::warn!("Modlight: the tray icon could not be created: {error}"))
            .ok()?;
        Some(Tray {
            icon: tray,
            built_for: signature(state),
            events,
        })
    }

    /// Rebuild the menu when the account or the language changed.
    pub fn update(&mut self, state: &AppState) {
        let now = signature(state);
        if now != self.built_for {
            self.built_for = now;
            self.icon.set_menu(Some(Box::new(build_menu(state))));
        }
    }

    /// What the menu asked for since the last frame.
    pub fn commands(&self) -> Vec<Command> {
        self.events
            .try_iter()
            .filter_map(|id| match id.as_str() {
                SIGN_IN | MODLIGHT_ACCOUNT => Some(Command::ShowSignIn),
                ACCOUNT_SETTINGS => Some(Command::ShowSettings(Some(
                    crate::settings::ACCOUNT_SECTION,
                ))),
                SIGN_OUT => Some(Command::SignOut),
                SETTINGS => Some(Command::ShowSettings(None)),
                SHOW => Some(Command::ShowMainWindow),
                _ => None,
            })
            .collect()
    }
}
