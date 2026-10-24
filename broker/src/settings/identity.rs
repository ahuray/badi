//! Stable subject identities: exact browser origins and canonical Linux app ids.

use std::cmp::Ordering;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::{
    LIBREOFFICE_WRITER_APP_ID, OriginScheme, TargetDescriptor, TargetKind, valid_linux_app_id,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdapter {
    Chromium,
}

/// Canonical subject order compares adapters by their wire name, as the
/// desktop CLI and settings panel sort them, independent of declaration order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxAdapter {
    Fcitx,
    Neovim,
    Obsidian,
    Shell,
    Libreoffice,
}

impl LinuxAdapter {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fcitx => "fcitx",
            Self::Neovim => "neovim",
            Self::Obsidian => "obsidian",
            Self::Shell => "shell",
            Self::Libreoffice => "libreoffice",
        }
    }
}

impl Ord for LinuxAdapter {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for LinuxAdapter {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebScheme {
    Http,
    Https,
}

impl WebScheme {
    #[must_use]
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StableIdentity {
    BrowserOrigin {
        adapter: BrowserAdapter,
        scheme: WebScheme,
        host: String,
        port: u16,
    },
    LinuxApp {
        adapter: LinuxAdapter,
        app_id: String,
    },
}

impl StableIdentity {
    pub fn browser_origin(
        adapter: BrowserAdapter,
        scheme: WebScheme,
        host: &str,
        port: Option<u16>,
    ) -> Result<Self, IdentityError> {
        let host = canonical_host(host)?;
        let port = port.unwrap_or_else(|| scheme.default_port());
        if port == 0 {
            return Err(IdentityError::InvalidPort);
        }
        Ok(Self::BrowserOrigin {
            adapter,
            scheme,
            host,
            port,
        })
    }

    pub fn linux_app(adapter: LinuxAdapter, app_id: &str) -> Result<Self, IdentityError> {
        if !valid_linux_app_id(app_id) {
            return Err(IdentityError::InvalidAppId);
        }
        Ok(Self::LinuxApp {
            adapter,
            app_id: app_id.to_owned(),
        })
    }

    pub fn from_target(target: &TargetDescriptor) -> Result<Self, IdentityError> {
        match target.kind {
            TargetKind::Browser
                if matches!(
                    target.app_id.as_str(),
                    "chromium" | "chromium-browser" | "brave-origin" | "zen"
                ) =>
            {
                let origin = target.origin.as_ref().ok_or(IdentityError::MissingOrigin)?;
                let scheme = match origin.scheme {
                    OriginScheme::Http => WebScheme::Http,
                    OriginScheme::Https => WebScheme::Https,
                    OriginScheme::File => return Err(IdentityError::UnsupportedScheme),
                };
                Self::browser_origin(BrowserAdapter::Chromium, scheme, &origin.host, origin.port)
            }
            TargetKind::DesktopApplication if target.origin.is_none() => {
                Self::linux_app(LinuxAdapter::Fcitx, &target.app_id)
            }
            TargetKind::Obsidian if target.origin.is_none() && target.app_id == "obsidian" => {
                Self::linux_app(LinuxAdapter::Obsidian, &target.app_id)
            }
            TargetKind::Terminal if target.origin.is_none() && target.app_id == "bash" => {
                Self::linux_app(LinuxAdapter::Shell, &target.app_id)
            }
            TargetKind::Neovim if target.origin.is_none() && target.app_id == "nvim" => {
                Self::linux_app(LinuxAdapter::Neovim, &target.app_id)
            }
            TargetKind::Libreoffice
                if target.origin.is_none() && target.app_id == LIBREOFFICE_WRITER_APP_ID =>
            {
                Self::linux_app(LinuxAdapter::Libreoffice, &target.app_id)
            }
            _ => Err(IdentityError::UnsupportedTarget),
        }
    }

    pub fn validate(&self) -> Result<(), IdentityError> {
        match self {
            Self::BrowserOrigin {
                adapter,
                scheme,
                host,
                port,
            } => {
                let canonical = Self::browser_origin(*adapter, *scheme, host, Some(*port))?;
                if &canonical == self {
                    Ok(())
                } else {
                    Err(IdentityError::NoncanonicalHost)
                }
            }
            Self::LinuxApp { adapter, app_id } => {
                let canonical = Self::linux_app(*adapter, app_id)?;
                if &canonical == self {
                    Ok(())
                } else {
                    Err(IdentityError::InvalidAppId)
                }
            }
        }
    }
}

fn canonical_host(raw: &str) -> Result<String, IdentityError> {
    if raw.is_empty() || raw.len() > 253 || !raw.is_ascii() {
        return Err(IdentityError::InvalidHost);
    }
    let without_brackets = raw
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(raw);
    if let Ok(address) = Ipv4Addr::from_str(without_brackets) {
        return Ok(address.to_string());
    }
    if let Ok(address) = Ipv6Addr::from_str(without_brackets) {
        return Ok(address.to_string());
    }
    let normalized = without_brackets.to_ascii_lowercase();
    let ambiguous_hex_address = normalized.strip_prefix("0x").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if normalized
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
        || ambiguous_hex_address
    {
        return Err(IdentityError::InvalidHost);
    }
    if normalized.ends_with('.')
        || normalized.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
    {
        return Err(IdentityError::InvalidHost);
    }
    Ok(normalized)
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum IdentityError {
    #[error("invalid_app_id")]
    InvalidAppId,
    #[error("invalid_host")]
    InvalidHost,
    #[error("invalid_port")]
    InvalidPort,
    #[error("missing_origin")]
    MissingOrigin,
    #[error("noncanonical_host")]
    NoncanonicalHost,
    #[error("unsupported_scheme")]
    UnsupportedScheme,
    #[error("unsupported_target")]
    UnsupportedTarget,
}
