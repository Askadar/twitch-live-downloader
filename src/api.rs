use std::str::FromStr;

use reqwest::{self, Client, StatusCode, header};
use serde_json::json;
use twitch_api::eventsub::EventType;

use crate::data::{
	Config, StreamData, StreamResponse, Token, UserData, UserResponse, ValidationResponse,
};
use crate::err::Error;

pub struct Api {
	c: reqwest::Client,
	token: Token,
	base: String,
	config: Config,
}

fn createClient(config: &Config, token: &Token) -> Result<Client, Error> {
	let defaultHeaders = reqwest::header::HeaderMap::from_iter(
		[
			(
				header::AUTHORIZATION,
				header::HeaderValue::from_str(&format!("Bearer {}", token.access_token))
					.expect("failed serializing header value"),
			),
			(
				header::HeaderName::from_str("Client-Id").unwrap(),
				header::HeaderValue::from_str(&config.clientId).unwrap(),
			),
			(
				header::CONTENT_TYPE,
				header::HeaderValue::from_str("application/json").unwrap(),
			),
		]
		.into_iter(),
	);
	let c = reqwest::Client::builder()
		.default_headers(defaultHeaders)
		.build()
		.expect("failed to create client");

	return Ok(c);
}

impl Api {
	pub fn init(token: Token, config: &Config) -> Self {
		let config = config.clone();
		let mut base = "https://api.twitch.tv/helix";
		if let Some(b) = &config.baseUrl {
			base = b;
		}

		let c = createClient(&config, &token).unwrap();

		Self {
			c,
			base: base.to_string(),
			token,
			config: config,
		}
	}

	pub async fn refreshToken(&mut self) -> Result<Token, Error> {
		let authClient = reqwest::Client::new();

		let req = authClient
			.post("https://id.twitch.tv/oauth2/token")
			.query(&[
				("client_id", self.config.clientId.as_str()),
				("client_secret", self.config.clientSecret.as_str()),
				("grant_type", "refresh_token"),
				("refresh_token", self.token.refresh_token.as_str()),
			])
			.build()
			.expect("failed to build token refresh req");
		let resp = authClient.execute(req).await.expect("failed token refresh");
		let status = resp.status();

		println!("[TKNR] [{}] {}", chrono::Utc::now(), &status);
		if status == StatusCode::UNAUTHORIZED {
			return Err(Error::ExpiredAuth);
		}

		let text = &resp.text().await.unwrap();
		let token: Token = serde_json::from_str(text).expect("failed to parse token response");
		self.token = token.clone();
		self.c = createClient(&self.config, &self.token).unwrap();

		Ok(token)
	}

	pub async fn validate(&self) -> Result<ValidationResponse, Error> {
		let req = self
			.c
			.get("https://id.twitch.tv/oauth2/validate")
			.build()
			.expect("failed to build login get");
		let resp = self.c.execute(req).await.expect("failed to validate token");
		let status = resp.status();
		let text = resp.text().await.unwrap();

		if status == StatusCode::UNAUTHORIZED {
			println!("[VLDF] [{}] {text}", chrono::Utc::now());
			return Err(Error::UnAuthorised);
		}

		let resp: ValidationResponse = serde_json::from_str(&text).unwrap();

		Ok(resp)
	}

	pub async fn getStream(&self, login: &[&str]) -> Result<Vec<StreamData>, Error> {
		let logins = login
			.iter()
			.map(|login| ("user_login", *login))
			.collect::<Vec<(&str, &str)>>();
		let req = self
			.c
			.get("https://api.twitch.tv/helix/streams")
			.query(&logins)
			.build()
			.expect("failed to build streams get");
		let resp = self
			.c
			.execute(req)
			.await
			.expect("failed to fetch streams data");
		let status = resp.status();
		if status == StatusCode::UNAUTHORIZED {
			return Err(Error::UnAuthorised);
		}
		let text = resp.text().await.unwrap();
		let json: StreamResponse = serde_json::from_str(&text).unwrap();

		Ok(json.data)
	}

	pub async fn getUsers(&self, login: &[String]) -> Result<Vec<UserData>, Error> {
		let logins = login
			.iter()
			.map(|login| ("login", login.as_str()))
			.collect::<Vec<(&str, &str)>>();
		let getLogin = self
			.c
			.get("https://api.twitch.tv/helix/users")
			.query(&logins)
			.build()
			.expect("failed to build login get");
		let loginResp = self
			.c
			.execute(getLogin)
			.await
			.expect("failed to fetch login data");
		let status = loginResp.status();
		if status == StatusCode::UNAUTHORIZED {
			return Err(Error::UnAuthorised);
		}
		let text = loginResp.text().await.unwrap();
		let json: UserResponse = serde_json::from_str(&text).unwrap();

		Ok(json.data)
	}

	pub async fn getUser(&self, login: &str) -> Result<UserData, Error> {
		let getLogin = self
			.c
			.get("https://api.twitch.tv/helix/users")
			.query(&[("login", login)])
			.build()
			.expect("failed to build login get");
		let loginResp = self
			.c
			.execute(getLogin)
			.await
			.expect("failed to fetch login data");
		let status = loginResp.status();
		if status == StatusCode::UNAUTHORIZED {
			return Err(Error::UnAuthorised);
		}
		let text = loginResp.text().await.unwrap();
		let json: UserResponse = serde_json::from_str(&text).unwrap();

		if let Some(user) = json.data.into_iter().nth(0) {
			Ok(user)
		} else {
			Err(Error::MissingUser)
		}
	}

	pub async fn subscribe(
		&self,
		session_id: &str,
		eType: EventType,
		condition: serde_json::Value,
	) -> Result<(), Error> {
		let body = json!({
			"type": eType,
			"version": 1,
			"condition": condition,
			"transport": {
				"method": "websocket",
				"session_id": session_id,
			}
		})
		.to_string();

		let sub = self
			.c
			.post(format!("{}/eventsub/subscriptions", self.base))
			.body(body)
			.build()
			.unwrap();
		let resp = self.c.execute(sub).await.unwrap();
		let status = resp.status();
		let text = resp.text().await.unwrap();
		if status != StatusCode::ACCEPTED {
			println!("[subF] [{}] {} {:?}", chrono::Utc::now(), status, text);
		} else {
			println!("[SUBK] [{}] {}", chrono::Utc::now(), condition);
		}

		Ok(())
	}
}
// https://api.twitch.tv/helix/eventsub/subscriptions
// http://127.0.0.1:8080/eventsub/subscriptions

pub async fn try_request<T, E, Fut, F: FnMut() -> Fut>(mut f: F, retries: i32) -> Result<T, E>
where
	Fut: Future<Output = Result<T, E>>,
{
	let mut count = 0;
	loop {
		let result = f().await;

		if result.is_ok() {
			break result;
		} else {
			if count > retries {
				break result;
			}
			count += 1;
		}
	}
}
