use serde::{Deserialize, Deserializer};

/// Top-level `/lol-gameflow/v1/session` payload (only the fields we need).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameflowSession {
    #[serde(default, deserialize_with = "deserialize_string")]
    pub phase: String,
    #[serde(default)]
    pub game_data: Option<GameData>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameData {
    #[serde(default, deserialize_with = "deserialize_i64")]
    pub game_id: i64,
    #[serde(default)]
    pub queue: Option<Queue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    #[serde(default, deserialize_with = "deserialize_i64")]
    pub id: i64,
    #[serde(default, deserialize_with = "deserialize_string")]
    pub game_mode: String,
}

impl GameflowSession {
    pub fn game_id(&self) -> i64 {
        self.game_data
            .as_ref()
            .map(|game_data| game_data.game_id)
            .unwrap_or_default()
    }

    pub fn queue(&self) -> Queue {
        self.game_data
            .as_ref()
            .and_then(|game_data| game_data.queue.clone())
            .unwrap_or_default()
    }
}

/// `/lol-champ-select/v1/session` payload (only the fields we need).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampSelectSession {
    #[serde(default, deserialize_with = "deserialize_optional_i64")]
    pub local_player_cell_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_vec")]
    pub my_team: Vec<TeamMember>,
    #[serde(default, deserialize_with = "deserialize_vec")]
    pub actions: Vec<Vec<Action>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    #[serde(default, deserialize_with = "deserialize_i64")]
    pub cell_id: i64,
    #[serde(default, deserialize_with = "deserialize_i64")]
    pub champion_id: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    #[serde(default, deserialize_with = "deserialize_i64")]
    pub actor_cell_id: i64,
    #[serde(default, deserialize_with = "deserialize_i64")]
    pub champion_id: i64,
    #[serde(default)]
    pub completed: bool,
    #[serde(default, rename = "type", deserialize_with = "deserialize_string")]
    pub kind: String,
}

impl ChampSelectSession {
    /// Returns the local player's current champion, preferring `myTeam`.
    ///
    /// Some LCU patches serialize cell/champion IDs as strings, so all IDs are
    /// parsed through the flexible deserializer below.
    pub fn local_champion_id(&self) -> Option<i64> {
        let local_player_cell_id = self.local_player_cell_id?;

        if let Some(member) = self
            .my_team
            .iter()
            .find(|member| member.cell_id == local_player_cell_id)
        {
            if member.champion_id > 0 {
                return Some(member.champion_id);
            }
        }

        self.actions
            .iter()
            .flatten()
            .rfind(|action| {
                action.actor_cell_id == local_player_cell_id
                    && action.kind == "pick"
                    && action.completed
                    && action.champion_id > 0
            })
            .map(|action| action.champion_id)
    }
}

/// Accepts a JSON number or numeric string, and treats null/missing as zero.
fn deserialize_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(deserialize_optional_i64(deserializer)?.unwrap_or_default())
}

fn deserialize_optional_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::Number(number)) => number.as_i64(),
        Some(serde_json::Value::String(text)) => text.parse().ok(),
        _ => None,
    })
}

fn deserialize_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gameflow_mayhem_session() {
        let json = br#"{
            "phase": "ChampSelect",
            "gameData": {
                "gameId": 123456,
                "queue": { "id": 2400, "gameMode": "KIWI" }
            }
        }"#;

        let session: GameflowSession = serde_json::from_slice(json).expect("session");
        assert_eq!(session.phase, "ChampSelect");
        assert_eq!(session.game_id(), 123456);
        assert_eq!(session.queue().id, 2400);
        assert_eq!(session.queue().game_mode, "KIWI");
    }

    #[test]
    fn missing_game_data_is_tolerated() {
        let session: GameflowSession =
            serde_json::from_str(r#"{"phase":"Lobby"}"#).expect("session");

        assert_eq!(session.game_id(), 0);
        assert_eq!(session.queue().id, 0);
    }

    #[test]
    fn parses_local_champion_from_my_team() {
        let json = br#"{
            "localPlayerCellId": 2,
            "myTeam": [
                { "cellId": 0, "championId": 1 },
                { "cellId": 2, "championId": 103 }
            ],
            "actions": []
        }"#;

        let session: ChampSelectSession = serde_json::from_slice(json).expect("session");
        assert_eq!(session.local_champion_id(), Some(103));
    }

    #[test]
    fn parses_ids_serialized_as_strings() {
        let json = br#"{
            "localPlayerCellId": "2",
            "myTeam": [{ "cellId": "2", "championId": "103" }],
            "actions": []
        }"#;

        let session: ChampSelectSession = serde_json::from_slice(json).expect("session");
        assert_eq!(session.local_champion_id(), Some(103));
    }

    #[test]
    fn falls_back_to_completed_pick_action() {
        let json = br#"{
            "localPlayerCellId": 1,
            "myTeam": [{ "cellId": 1, "championId": 0 }],
            "actions": [
                [
                    { "actorCellId": 1, "championId": 99, "completed": true, "type": "pick" }
                ]
            ]
        }"#;

        let session: ChampSelectSession = serde_json::from_slice(json).expect("session");
        assert_eq!(session.local_champion_id(), Some(99));
    }

    #[test]
    fn returns_none_when_no_champion_is_selected() {
        let json = br#"{
            "localPlayerCellId": 1,
            "myTeam": [{ "cellId": 1, "championId": 0 }],
            "actions": []
        }"#;

        let session: ChampSelectSession = serde_json::from_slice(json).expect("session");
        assert_eq!(session.local_champion_id(), None);
    }

    #[test]
    fn missing_local_cell_does_not_guess_cell_zero() {
        let json = br#"{
            "myTeam": [{ "cellId": 0, "championId": 103 }],
            "actions": []
        }"#;

        let session: ChampSelectSession = serde_json::from_slice(json).expect("session");
        assert_eq!(session.local_champion_id(), None);
    }

    #[test]
    fn null_arrays_and_strings_are_tolerated() {
        let json = br#"{
            "localPlayerCellId": 1,
            "myTeam": null,
            "actions": null
        }"#;

        let session: ChampSelectSession = serde_json::from_slice(json).expect("session");
        assert_eq!(session.local_champion_id(), None);

        let gameflow: GameflowSession = serde_json::from_str(
            r#"{"phase":null,"gameData":{"queue":{"id":null,"gameMode":null}}}"#,
        )
        .expect("gameflow");
        assert_eq!(gameflow.phase, "");
        assert_eq!(gameflow.queue().id, 0);
        assert_eq!(gameflow.queue().game_mode, "");
    }
}
