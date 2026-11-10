use std::collections::HashMap;

use serde::Deserialize;

/// Base URL for ARAMKit champion pages.
pub const ARAMKIT_CHAMPION_BASE: &str = "https://aramkit.cn/zh-CN/champions/";

/// Queue ID for ARAM: Mayhem (海克斯大乱斗).
pub const MAYHEM_QUEUE_ID: i64 = 2400;

/// Internal game-mode identifier used by ARAM: Mayhem.
pub const MAYHEM_GAME_MODE: &str = "KIWI";

/// One entry from the client's champion-summary asset.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChampionSummaryEntry {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChampionInfo {
    pub id: i64,
    pub name: String,
    pub alias: String,
}

/// Champion ID to localized name/alias lookup, loaded from the League client.
#[derive(Debug, Clone, Default)]
pub struct ChampionCatalog {
    by_id: HashMap<i64, ChampionInfo>,
}

impl ChampionCatalog {
    pub fn from_entries(entries: impl IntoIterator<Item = ChampionSummaryEntry>) -> Self {
        let by_id = entries
            .into_iter()
            .filter(|entry| entry.id > 0 && !entry.alias.is_empty())
            .map(|entry| {
                (
                    entry.id,
                    ChampionInfo {
                        id: entry.id,
                        name: entry.name,
                        alias: entry.alias,
                    },
                )
            })
            .collect();
        Self { by_id }
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let entries: Vec<ChampionSummaryEntry> = serde_json::from_slice(bytes)?;
        Ok(Self::from_entries(entries))
    }

    pub fn get(&self, champion_id: i64) -> Option<&ChampionInfo> {
        self.by_id.get(&champion_id)
    }

    pub fn name_of(&self, champion_id: i64) -> Option<&str> {
        self.get(champion_id).map(|info| info.name.as_str())
    }
}

/// Converts a Data Dragon-style alias into the slug ARAMKit uses.
///
/// Examples: `MonkeyKing` -> `monkeyking`, `K'Sante` -> `ksante`.
pub fn champion_slug(alias: &str) -> Option<String> {
    let slug: String = alias
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    (!slug.is_empty()).then_some(slug)
}

/// Builds the ARAMKit champion URL, or `None` when the alias is unusable.
pub fn champion_url_for_alias(alias: &str) -> Option<String> {
    champion_slug(alias).map(|slug| format!("{ARAMKIT_CHAMPION_BASE}{slug}"))
}

/// Queue IDs and internal mode names can both identify ARAM: Mayhem.
pub fn is_mayhem_queue(queue_id: i64, game_mode: &str) -> bool {
    queue_id == MAYHEM_QUEUE_ID || game_mode.eq_ignore_ascii_case(MAYHEM_GAME_MODE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_and_strips_punctuation() {
        assert_eq!(champion_slug("MonkeyKing"), Some("monkeyking".to_owned()));
        assert_eq!(champion_slug("K'Sante"), Some("ksante".to_owned()));
        assert_eq!(champion_slug("Vel'Koz"), Some("velkoz".to_owned()));
        assert_eq!(champion_slug("Ahri"), Some("ahri".to_owned()));
    }

    #[test]
    fn empty_alias_has_no_url() {
        assert_eq!(champion_slug(""), None);
        assert_eq!(champion_url_for_alias("---"), None);
    }

    #[test]
    fn champion_url_uses_zh_cn_route() {
        assert_eq!(
            champion_url_for_alias("Ahri"),
            Some("https://aramkit.cn/zh-CN/champions/ahri".to_owned())
        );
    }

    #[test]
    fn mayhem_is_matched_by_queue_or_mode() {
        assert!(is_mayhem_queue(2400, ""));
        assert!(is_mayhem_queue(0, "kiwi"));
        assert!(!is_mayhem_queue(450, "ARAM"));
    }

    #[test]
    fn catalog_parses_entries_and_skips_none() {
        let json = br#"[
            {"id": -1, "name": "None", "alias": "None"},
            {"id": 103, "name": "Ahri", "alias": "Ahri"},
            {"id": 121, "name": "Kha'Zix", "alias": "Khazix"}
        ]"#;

        let catalog = ChampionCatalog::from_json(json).expect("catalog");
        assert_eq!(catalog.name_of(103), Some("Ahri"));
        assert_eq!(catalog.name_of(121), Some("Kha'Zix"));
        assert_eq!(catalog.get(-1), None);
    }
}
