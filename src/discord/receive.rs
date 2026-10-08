use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveMode {
    #[default]
    Normal,
    Whitelist,
    Auto,
}

#[derive(Clone, Copy, Default, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BotStatus {
    #[default]
    Disabled,
    Connecting,
    Unknown,
    Dnd,
    Online,
    Idle,
    Offline,
    Error,
    MissingGuild,
    MissingUser,
    AccountMismatch,
}

struct State {
    mode: ReceiveMode,
    bot: BotStatus,
    bot_user: String,
    rpc_user: Option<String>,
    channels: HashSet<String>,
}

#[derive(Clone)]
pub struct ReceiveControl(Arc<RwLock<State>>);
#[derive(Clone)]
pub struct NotificationGate {
    control: ReceiveControl,
    channel: String,
    raw_message: bool,
}
impl ReceiveControl {
    pub fn new(config: &Config) -> Self {
        Self(Arc::new(RwLock::new(State {
            mode: config.receive_mode,
            bot: BotStatus::Disabled,
            bot_user: config.bot_user_id.clone(),
            rpc_user: None,
            channels: config.whitelist_channel_ids.iter().cloned().collect(),
        })))
    }
    pub fn configure(&self, config: &Config) {
        if let Ok(mut state) = self.0.write() {
            state.mode = config.receive_mode;
            state.bot_user = config.bot_user_id.clone();
            state.rpc_user = None;
            state.bot = BotStatus::Disabled;
            state.channels = config.whitelist_channel_ids.iter().cloned().collect();
        }
    }
    pub fn set_mode(&self, mode: ReceiveMode) {
        if let Ok(mut state) = self.0.write() {
            state.mode = mode;
        }
    }
    pub fn set_rpc_user(&self, user: Option<String>) {
        if let Ok(mut state) = self.0.write() {
            state.rpc_user = user;
        }
    }
    pub fn set_bot_status(&self, status: BotStatus) {
        if let Ok(mut state) = self.0.write() {
            state.bot = status;
        }
    }
    pub fn mode(&self) -> ReceiveMode {
        self.0
            .read()
            .map(|s| s.mode)
            .unwrap_or(ReceiveMode::Whitelist)
    }
    pub fn bot_status(&self) -> BotStatus {
        self.0
            .read()
            .map(|s| {
                if s.mode == ReceiveMode::Auto
                    && s.rpc_user.as_ref().is_some_and(|id| id != &s.bot_user)
                {
                    BotStatus::AccountMismatch
                } else {
                    s.bot
                }
            })
            .unwrap_or(BotStatus::Unknown)
    }
    pub fn whitelist_only(&self) -> bool {
        self.0.read().map(|s| restricted(&s)).unwrap_or(true)
    }
    pub fn gate(&self, channel: Option<&str>, raw_message: bool) -> NotificationGate {
        NotificationGate {
            control: self.clone(),
            channel: channel.unwrap_or("").into(),
            raw_message,
        }
    }
}
fn restricted(state: &State) -> bool {
    match state.mode {
        ReceiveMode::Normal => false,
        ReceiveMode::Whitelist => true,
        ReceiveMode::Auto => {
            !(state.rpc_user.as_deref() == Some(state.bot_user.as_str())
                && !state.bot_user.is_empty()
                && matches!(state.bot, BotStatus::Online | BotStatus::Idle))
        }
    }
}
impl NotificationGate {
    pub fn allows(&self) -> bool {
        self.control
            .0
            .read()
            .map(|s| {
                if restricted(&s) {
                    s.channels.contains(&self.channel)
                } else {
                    !self.raw_message
                }
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_and_auto_gate_incoming_and_queued_messages_conservatively() {
        let cfg = Config {
            whitelist_channel_ids: vec!["100".into()],
            bot_user_id: "200".into(),
            ..Config::default()
        };
        let control = ReceiveControl::new(&cfg);
        let allowed = control.gate(Some("100"), false);
        let other = control.gate(Some("101"), false);
        let raw = control.gate(Some("100"), true);
        assert!(other.allows());
        assert!(!raw.allows());
        control.set_mode(ReceiveMode::Whitelist);
        assert!(allowed.allows());
        assert!(raw.allows());
        assert!(!other.allows());
        assert!(!control.gate(None, false).allows());
        control.set_mode(ReceiveMode::Auto);
        control.set_bot_status(BotStatus::Online);
        assert!(!other.allows()); // Never trust a different/unidentified RPC account.
        control.set_rpc_user(Some("200".into()));
        assert!(other.allows());
        assert!(!raw.allows());
        for status in [
            BotStatus::Dnd,
            BotStatus::Offline,
            BotStatus::Unknown,
            BotStatus::Connecting,
            BotStatus::Error,
            BotStatus::MissingGuild,
            BotStatus::MissingUser,
        ] {
            control.set_bot_status(status);
            assert!(!other.allows());
            assert!(raw.allows());
        }
        control.set_bot_status(BotStatus::Idle);
        assert!(other.allows());
        control.set_rpc_user(Some("201".into()));
        assert!(!other.allows());
        assert_eq!(control.bot_status(), BotStatus::AccountMismatch);
        control.set_mode(ReceiveMode::Normal);
        assert!(other.allows());
        assert!(!raw.allows());
    }
}
