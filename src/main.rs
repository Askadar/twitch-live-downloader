#![allow(non_snake_case)]

use futures::StreamExt;
use log::{debug, error, info, trace, warn};
use std::{
	fs,
	path::PathBuf,
	process::Stdio,
	sync::{Arc, atomic::Ordering},
	time::Duration,
};
use tokio::sync::Mutex;

pub mod api;
pub mod data;
pub mod err;
pub mod socket;
pub mod token;

use data::InternalMessage;
use twitch_api::eventsub::EventType;

use crate::{
	api::Api,
	data::{Config, ValidationResponse},
};

fn setup_logger(config: &Config) -> Result<(), fern::InitError> {
	let mut chatChain = fern::Dispatch::new();

	let mut chatRoot = "chat".to_string();
	if let Some(specifiedRoot) = config.chatRoot.clone() {
		chatRoot = specifiedRoot;
	}
	fs::create_dir_all(&chatRoot)?;

	for broadcaster in &config.broadcasters {
		let mut path = PathBuf::from(&chatRoot).join(broadcaster);
		path.set_extension("log");

		chatChain = chatChain.chain(
			fern::Dispatch::new()
				.level(log::LevelFilter::Off)
				.level_for(format!("ld::chat::{broadcaster}"), log::LevelFilter::Trace)
				.format(|out, message, record| {
					out.finish(format_args!(
						"[{} {}] {}",
						chrono::Utc::now(),
						record.target(),
						message
					))
				})
				.chain(fern::log_file(path)?),
		);
	}

	let loggerChain = fern::Dispatch::new()
		.format(|out, message, record| {
			out.finish(format_args!(
				"[{} {}] [{}] {}",
				chrono::Utc::now(),
				match record.level() {
					log::Level::Error => "ERROR",
					log::Level::Warn => "WARN_",
					log::Level::Info => "INFO_",
					log::Level::Debug => "DEBUG",
					log::Level::Trace => "TRACE",
				},
				record.target(),
				message
			))
		})
		.chain(
			fern::Dispatch::new()
				.level(log::LevelFilter::Info)
				.chain(std::io::stdout()),
		)
		.chain(
			fern::Dispatch::new()
				.level(log::LevelFilter::Off)
				.level_for("ld", log::LevelFilter::Trace)
				.chain(fern::log_file("live-downloader.log")?),
		)
		.chain(
			fern::Dispatch::new()
				.level(log::LevelFilter::Trace)
				.level_for("ld", log::LevelFilter::Off)
				.chain(fern::log_file("vendor.log")?),
		);

	fern::Dispatch::new()
		.chain(chatChain)
		.chain(loggerChain)
		.apply()?;

	Ok(())
}

async fn validateAndRefreshToken(
	api: &std::sync::Arc<tokio::sync::Mutex<api::Api>>,
) -> Option<ValidationResponse> {
	info!("[VLDT] ");
	let mut apilock: tokio::sync::MutexGuard<Api> = api.lock().await;
	match apilock.validate().await {
		Err(err) => match err {
			crate::err::Error::UnAuthorised => {
				let token = apilock.refreshToken().await.unwrap();
				token::writeRefreshToken(&token).await.unwrap();
				None
			}
			_ => {
				error!("[VLDT] skipping err {:?} for validation", err);
				None
			}
		},
		Ok(resp) => Some(resp),
	}
}

enum ThreadType {
	MainSocket,
	Validation,
	Download(String),
}

struct Thread {
	active: bool,
	label: ThreadType,
	id: u32,
	handle: tokio::task::JoinHandle<()>,
}

pub fn id() -> u32 {
	static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
	return COUNTER.fetch_add(1, Ordering::Relaxed);
}

impl Thread {
	fn new(label: ThreadType, handle: tokio::task::JoinHandle<()>) -> Arc<Self> {
		Arc::new(Thread {
			active: true,
			label: label,
			handle: handle,
			id: id(),
		})
	}
}
struct ThreadSwap {
	from: Arc<Thread>,
	to: Arc<Thread>,
}

#[tokio::main]
async fn main() {
	let threadPool: Arc<Mutex<Vec<Arc<Thread>>>> = Arc::new(Mutex::new(Vec::new()));

	let configPath = std::env::current_dir().unwrap().join("config.json");
	let config: Config = serde_json::from_slice(&std::fs::read(configPath).unwrap()).unwrap();
	setup_logger(&config).expect("Failed to setup logging chain");

	let (tx, mut rx) = tokio::sync::broadcast::channel::<InternalMessage>(32);
	let wsTx = tx.clone();
	let apiTx = tx.clone();
	let token = token::fetchToken(&config).await.unwrap();

	let rootPath = PathBuf::from(&config.root);

	let api: std::sync::Arc<tokio::sync::Mutex<api::Api>> = std::sync::Arc::new(
		tokio::sync::Mutex::new(api::Api::init(token.clone(), &config)),
	);

	let mut socketUrl = "wss://eventsub.wss.twitch.tv/ws?keepalive_timeout_seconds=300".to_string();
	if let Some(s) = &config.socketUrl {
		socketUrl = s.clone();
	}

	let socket = Arc::new(socket::Client { tx: wsTx });
	let mut mainLock = threadPool.lock().await;

	let validateApi = api.clone();
	mainLock.push(Thread::new(
		ThreadType::Validation,
		tokio::spawn(async move {
			let mut prevResult: Option<ValidationResponse> = None;
			loop {
				let mut duration = Duration::from_secs(3600);
				if let Some(val) = prevResult {
					duration = Duration::from_secs(std::cmp::min(val.expires_in + 1, 3600));
				}
				tokio::time::sleep(duration).await;

				prevResult = validateAndRefreshToken(&validateApi).await
			}
		}),
	));

	validateAndRefreshToken(&api.clone()).await;

	let users = api
		.lock()
		.await
		.getUsers(
			&config
				.broadcasters
				.iter()
				.map(|br| br.as_str())
				.collect::<Vec<&str>>(),
		)
		.await
		.expect("failed to get user details");

	let account = api
		.lock()
		.await
		.getUser(&config.account)
		.await
		.expect("failed to get account details");
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

	let switch: Arc<Mutex<Option<ThreadSwap>>> = Arc::new(Mutex::new(None));

	let tsSocket = socket.clone();
	mainLock.push(Thread::new(
		ThreadType::MainSocket,
		tokio::spawn(async move {
			use tungstenite::Message;
			let (tungstenSocketStream, _connectionResponse) = tokio_tungstenite::connect_async(socketUrl)
				.await
				.expect("WebSocket connection failed to initialise");
			tungstenSocketStream
				.for_each(|msg| async {
					// let messageType = determineType(&msg);
					let t: Result<Option<tungstenite::Utf8Bytes>, tungstenite::Error> =
						msg.map(|msg| match msg {
							Message::Text(val) => Some(val),
							Message::Frame(val) => {
								info!("unexpected frame on socket: {:?}", val);
								None
							}
							Message::Binary(bin) => {
								trace!("received binary on websocket: {:?}", bin);
								None
							}
							Message::Ping(val) => {
								trace!("ping: {:?}", val);
								None
							}
							Message::Pong(val) => {
								trace!("pong: {:?}", val);
								None
							}
							Message::Close(val) => {
								warn!("got close frame: {:?}", val);
								None
							}
						});

					match t {
						Ok(val) => match val {
							Some(text) => {
								log::trace!("[MSGA] {:?}", text);
								tsSocket.processFrame(text).unwrap();
							}
							None => {
								// noop
							}
						},
						Err(detail) => {
							error!("{}", detail);
						}
					}
				})
				.await
		}),
	));
	drop(mainLock);

	debug!(
		"[REDY] {}",
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
	// handle streamlink output and/or investigate custom downloader since priority is shifted toward reliably getting everything of a vod, and streamlink drops segments on flaky connection

	loop {
		use InternalMessage::{Chat, Debug, DontHandle, Init, Reconnect, StreamLive, StreamStop};

		match rx.recv().await.unwrap() {
			Init { session } => {
				let val = switch.lock().await.take();
				if let Some(swap) = val {
					info!("[RNIT] session: {session}");

					// TODO maybe bring this back later, would have to think about threadpool signature
					// tungstenSocketStream
					// 	.close(Some(tungstenite::protocol::CloseFrame {
					// 		code: tungstenite::protocol::frame::coding::CloseCode::Normal,
					// 		reason: Utf8Bytes::from_static(""),
					// 	}))
					// 	.await
					// 	.expect("failed to close socket on reconnect");

					let mut pool = threadPool.lock().await;
					let pos = pool.iter().position(|t| t.id == swap.from.id).unwrap();
					pool.remove(pos);

					// threadPool.iter_mut().find
				} else {
					info!("[INIT] session: {session}");

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

							apilock
								.subscribe(
									&session,
									EventType::ChannelChatMessage,
									serde_json::json!({ "broadcaster_user_id": user.id, "user_id": account.id }),
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
				info!("[STRT] channel: {channel}");

				let mut path = rootPath
					.join(&channel)
					.join(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
				path.set_extension(&"mp4");
				let token = config.streamlinkToken.clone();

				threadPool.lock().await.push(Thread::new(
					ThreadType::Download(channel.clone()),
					tokio::spawn(async move {
						let path = path.to_str().unwrap();
						// switch ytdlp --add-headers "Authorization:OAuth {token}" "twitch.tv/negnasu"
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

						info!("[DLDN] ");

						()
					}),
				));
			}
			StreamStop { channel } => {
				trace!("[RNIT] channel: {channel}");
			}

			Debug { info } => {
				debug!("[DEBG] {}", info);
			}

			Chat { msg, channel } => {
				trace!(target: &format!("ld::chat::{}", channel), "[CHAT] {:?}", msg);
			}

			Reconnect { session, url } => {
				info!("[RCNT] session: {session}; url: {url}");

				let tsSocket = socket.clone();

				let newSocketThread = Thread::new(
					ThreadType::MainSocket,
					tokio::spawn(async move {
						let (tungstenSocketStream, _connectionResponse) = tokio_tungstenite::connect_async(url)
							.await
							.expect("WebSocket connection failed to initialise");

						tungstenSocketStream
							.for_each(|msg| async {
								let t = msg.and_then(|msg| msg.into_text());
								match t {
									Ok(text) => {
										log::trace!("[MSGA] {:?}", text);
										tsSocket.processFrame(text).unwrap();
									}

									Err(detail) => {
										error!("{}", detail);
										()
									}
								}
							})
							.await
					}),
				);
				let mut pool = threadPool.lock().await;
				let oldMainSocket = pool
					.iter()
					.find(|t| matches!(t.label, ThreadType::MainSocket));

				switch.lock().await.replace(ThreadSwap {
					from: oldMainSocket
						.expect("failed to find previous main socket thread")
						.clone(),
					to: newSocketThread.clone(),
				});

				pool.push(newSocketThread);
			}

			DontHandle => {
				// TODO maybe add details if wanna track it proper
				trace!(target: "ld::unhandled", "received an unexpected message")
				// noop
			}
		};
	}
}
