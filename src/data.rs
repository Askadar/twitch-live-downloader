use serde;

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct Token {
	pub access_token: String,
	pub refresh_token: String,
	pub expires_in: u32,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct Config {
	pub clientId: String,
	pub clientSecret: String,
	pub broadcasters: Vec<String>,
	pub streamlinkToken: String,
	pub root: String,
	pub socketUrl: Option<String>,
	pub baseUrl: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
pub struct DeviceCodeResponse {
	pub device_code: String,
	pub verification_uri: String,
}

#[derive(serde::Deserialize, Debug)]
pub struct UserData {
	pub id: String,
	pub login: String,
}

#[derive(serde::Deserialize, Debug)]
pub struct UserResponse {
	pub data: Vec<UserData>,
}

#[derive(serde::Deserialize, Debug)]
pub struct StreamData {
	pub id: String,
	pub user_id: String,
	pub user_login: String,
	pub game_name: String,
	#[serde(rename = "type")]
	pub stream_type: String,
	pub title: String,
	pub started_at: chrono::DateTime<chrono::Utc>,
}

#[derive(serde::Deserialize, Debug)]
pub struct StreamResponse {
	pub data: Vec<StreamData>,
}

#[derive(Debug, Clone)]
pub enum InternalMessage {
	Init { session: String },
	StreamLive { channel: String },
	StreamStop { channel: String },

	Debug { info: String },
	Reconnect { session: String, url: String },

	DontHandle,
}
