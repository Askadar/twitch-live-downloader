#![allow(non_snake_case)]

use std::{path::PathBuf, process::Stdio, time::Duration};

use ezsockets::{ClientConfig, CloseFrame};

pub mod api;
pub mod data;
pub mod err;
pub mod socket;
pub mod token;

use data::InternalMessage;
use tokio::task::JoinHandle;
use twitch_api::eventsub::EventType;

use crate::data::{Config, ValidationResponse};

async fn validateAndRefreshToken(
	api: &std::sync::Arc<tokio::sync::Mutex<api::Api>>,
) -> Option<ValidationResponse> {
	println!("[VLDT] [{}]", chrono::Utc::now());
	let mut apilock = api.lock().await;
	match apilock.validate().await {
		Err(err) => match err {
			crate::err::Error::UnAuthorised => {
				let token = apilock.refreshToken().await.unwrap();
				token::writeRefreshToken(&token).await.unwrap();
				None
			}
			_ => {
				println!("not handling err {:?} for validation", err);
				None
			}
		},
		Ok(resp) => Some(resp),
	}
}

#[tokio::main]
async fn main() {
	let (tx, mut rx) = tokio::sync::broadcast::channel::<InternalMessage>(32);
	let wsTx = tx.clone();
	let apiTx = tx.clone();

	let configPath = std::env::current_dir().unwrap().join("config.json");

	let config: Config = serde_json::from_slice(&std::fs::read(configPath).unwrap()).unwrap();
	let token = token::fetchToken(&config).await.unwrap();

	let rootPath = PathBuf::from(&config.root);

	let api: std::sync::Arc<tokio::sync::Mutex<api::Api>> = std::sync::Arc::new(
		tokio::sync::Mutex::new(api::Api::init(token.clone(), &config)),
	);

	let mut socketUrl = "wss://eventsub.wss.twitch.tv/ws";
	if let Some(s) = &config.socketUrl {
		socketUrl = s.as_str();
	}
	let socketConfig = ClientConfig::new(socketUrl)
		// .bearer(token.access_token)
		.max_reconnect_attempts(1)
		.max_initial_connect_attempts(1)
		.query_parameter("keepalive_timeout_seconds", "180");
	let (mut handle, future) = ezsockets::connect(
		move |_client| socket::Client {
			tx: wsTx,
			// rx: rx.clone(),
		},
		socketConfig,
	)
	.await;
	let mut newHandles: Option<(
		ezsockets::client::Client<socket::Client>,
		tokio::task::JoinHandle<()>,
	)> = None;

	let mut socketThread =
		tokio::spawn(async move { future.await.inspect_err(|e| println!("e {:?}", e)).unwrap() });
	let validateApi = api.clone();
	let _validateThread = tokio::spawn(async move {
		let mut prevResult: Option<ValidationResponse> = None;
		loop {
			let mut duration = Duration::from_secs(3600);
			if let Some(val) = prevResult {
				duration = Duration::from_secs(std::cmp::min(val.expires_in + 1, 3600));
			}
			tokio::time::sleep(duration).await;

			prevResult = validateAndRefreshToken(&validateApi).await
		}
	});

	validateAndRefreshToken(&api.clone()).await;

	// would prolly want an arc and or mutex on this
	let mut pool: Vec<JoinHandle<()>> = Vec::new();

	let users = api
		.lock()
		.await
		.getUsers(&config.broadcasters)
		.await
		.expect("failed to get user details");
	let streams = api
		.lock()
		.await
		.getStream(
			&config
				.broadcasters
				.iter()
				.map(String::as_str)
				.collect::<Vec<_>>(),
		)
		.await
		.unwrap();
	streams
		.iter()
		.filter(|s| s.stream_type == "live")
		.for_each(|s| {
			apiTx
				.send(InternalMessage::StreamLive {
					channel: s.user_login.clone(),
				})
				.expect("failed to send stream status update");
		});

	println!(
		"[REDY] [{}] {}",
		chrono::Utc::now(),
		users
			.iter()
			.map(|u| format!("{}: {}", u.login, u.id))
			.collect::<Vec<_>>()
			.join("; ")
	);

	// !TODO
	// bitconnect!
	// reconnect is still mildly broken
	//   more logging added, need verification what went wrong
	// also need to add token refresh on api fails, kinda tricky unfortunately
	//   hacked with checking expiration and scheduling next validation right after expiration
	// actually handle all the hanging threads + design overall concurrency system
	//   likely main thread blocking on resolving of monitor (message handling), and misc threads (socket, validator, etc)

	loop {
		use InternalMessage::{Debug, DontHandle, Init, Reconnect, StreamLive, StreamStop};

		match rx.recv().await.unwrap() {
			Init { session } => {
				if let Some((newHandle, newSocketThread)) = newHandles {
					println!("[RNIT] [{}] session: {:?}", chrono::Utc::now(), session);

					handle
						.close(Some(CloseFrame {
							code: ezsockets::CloseCode::Normal,
							reason: "".into(),
						}))
						.expect("failed to close socket on reconnect");

					socketThread.await.unwrap();
					socketThread = newSocketThread;
					handle = newHandle;
					newHandles = None;
				} else {
					println!("[INIT] [{}] session: {:?}", chrono::Utc::now(), session);

					let futures = users
						.iter()
						.map(|user| async {
							let apilock = api.lock().await;
							apilock
								.subscribe(
									&session,
									EventType::StreamOnline,
									serde_json::json!({ "broadcaster_user_id": user.id}),
								)
								.await
								.expect(&format!("failed to sub to {}", &user.login));
							tokio::time::sleep(Duration::from_millis(400)).await;
							apilock
								.subscribe(
									&session,
									EventType::StreamOffline,
									serde_json::json!({ "broadcaster_user_id": user.id}),
								)
								.await
								.expect(&format!("failed to sub to {}", &user.login));
							tokio::time::sleep(Duration::from_millis(400)).await;
						})
						.collect::<Vec<_>>();
					futures::future::join_all(futures).await;
				}
			}

			StreamLive { channel } => {
				println!("[STRT] [{}] ch: {:?}", chrono::Utc::now(), channel);
				let mut path = rootPath
					.join(&channel)
					.join(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
				path.set_extension(&"mp4");
				let token = config.streamlinkToken.clone();

				pool.push(tokio::spawn(async move {
					let path = path.to_str().unwrap();
					std::process::Command::new("streamlink")
						.args(&[
							"--http-header",
							&format!("Authorization=OAuth {}", token),
							"--hls-live-restart",
							"--hls-playlist-reload-time",
							"3",
							"--twitch-supported-codecs",
							"h264,h265,av1",
							"--retry-streams",
							"5",
							&format!("twitch.tv/{}", channel),
							"best",
							"-o",
							path,
						])
						.stdout(Stdio::inherit())
						.stderr(Stdio::inherit())
						.spawn()
						.expect("there was an error processing streamlink");
					()
				}))

				//  subprocess.call(["streamlink", '--http-header', 'Authorization=OAuth ' + self.oauth_tok_private, "--hls-live-restart", "--hls-playlist-reload-time", "3", "--twitch-supported-codecs", "h264,h265,av1", "--hls-live-restart", "--retry-streams", str(self.refresh), "twitch.tv/" + self.username, self.quality] + self.debug_cmd + ["-o", recorded_filename])
			}
			StreamStop { channel } => {
				println!("[STPD] [{}] ch: {:?}", chrono::Utc::now(), channel);
			}

			Debug { info } => {
				println!("[DEBG] [{}] {}", chrono::Utc::now(), info);
			}

			Reconnect { session, url } => {
				println!("[RCNT] [{}] {} - {}", chrono::Utc::now(), session, url);
				let wsTx = tx.clone();
				// recreateSocket(url, session)
				let newConfig = ClientConfig::new(url.as_str())
					.max_reconnect_attempts(3)
					.max_initial_connect_attempts(3)
					.query_parameter("keepalive_timeout_seconds", "180");
				let (_handle, future) = ezsockets::connect(
					move |_client| socket::Client {
						tx: wsTx,
						// rx: rx.clone(),
					},
					newConfig,
				)
				.await;
				newHandles = Some((
					_handle,
					tokio::spawn(async move { future.await.inspect_err(|e| println!("e {:?}", e)).unwrap() }),
				));
			}

			DontHandle => {
				// noop
			}
		};
	}
}
