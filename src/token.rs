use std::{path::PathBuf, time::Duration};

use reqwest::StatusCode;

use crate::{
	data::{Config, DeviceCodeResponse, Token},
	err::Error,
};

fn tokenPath() -> PathBuf {
	return std::env::current_dir().unwrap().join("token.json");
}

pub async fn getDeviceToken(clientId: &str, clientSecret: &str) -> Result<Token, Error> {
	let scopes = "user:read:chat";
	let authClient = reqwest::Client::new();
	let req = authClient
		.post("https://id.twitch.tv/oauth2/device")
		.query(&[
			("client_id", clientId),
			("client_secret", clientSecret),
			("scopes", &scopes),
		])
		.build()
		.expect("failed to build auth request");

	let deviceResp: DeviceCodeResponse = serde_json::from_str(
		&authClient
			.execute(req)
			.await
			.expect("failed to retrieve auth token")
			.text()
			.await
			.unwrap(),
	)
	.unwrap();
	println!("{:?}", &deviceResp);
	println!("follow the link to auth: {}", &deviceResp.verification_uri);

	loop {
		tokio::time::sleep(Duration::from_secs(5)).await;
		let deviceConfirm = authClient
			.post("https://id.twitch.tv/oauth2/token")
			.query(&[
				("client_id", clientId),
				("client_secret", clientSecret),
				("scopes", &scopes),
				("device_code", &deviceResp.device_code),
				("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
			])
			.build()
			.expect("failed to build device confirm");

		let result = match authClient.execute(deviceConfirm).await {
			Err(e) => {
				println!("err fetching auth {:?}", e);
				None
			}
			Ok(r) => match r.status() {
				StatusCode::BAD_REQUEST => None,
				_ => Some(serde_json::from_str(&r.text().await.unwrap()).unwrap()),
			},
		};

		if let Some(token) = result {
			return Ok(token);
		}
	}
}

pub async fn fetchToken(config: &Config) -> Result<Token, Error> {
	let mut token = std::fs::read(tokenPath())
		.map(|s| serde_json::from_slice::<Token>(&s).unwrap())
		.ok();
	if token.is_none() {
		token = getDeviceToken(&config.clientId, &config.clientSecret)
			.await
			.ok();
		if let Some(content) = &token {
			let text = serde_json::to_string_pretty(content).unwrap();
			std::fs::write(tokenPath(), text).expect("Failed to save token");
		};
	}
	let token = token.expect("Failed to retrieve token");

	Ok(token)
}

pub async fn writeRefreshToken(token: &Token) -> Result<(), Error> {
	std::fs::write(tokenPath(), serde_json::to_string_pretty(token).unwrap())
		.expect("Failed to save token");

	Ok(())
}
