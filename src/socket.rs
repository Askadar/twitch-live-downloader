use async_trait::async_trait;
use twitch_api::eventsub::{Event, EventsubWebsocketData, Message, Payload};

use crate::data::InternalMessage;

pub struct Client {
	pub tx: tokio::sync::broadcast::Sender<InternalMessage>,
	// pub rx: tokio::sync::broadcast::Receiver<InternalMessage>,
}

#[async_trait]
impl ezsockets::ClientExt for Client {
	type Call = ();

	async fn on_text(&mut self, text: ezsockets::Utf8Bytes) -> Result<(), ezsockets::Error> {
		// twitch_api::eventsub::Event::parse_websocket
		// println!("[msgA] {} {:?}", chrono::Utc::now(), text);

		let t = twitch_api::eventsub::Event::parse_websocket(&text).unwrap();
		match t {
			EventsubWebsocketData::Notification { payload, .. } => match payload {
				Event::StreamOnlineV1(Payload {
					message: Message::Notification(data),
					..
				}) => self
					.tx
					.send(InternalMessage::StreamLive {
						channel: data.broadcaster_user_login.to_string(),
					})
					.expect("failed to broadcast stream live"),
				Event::StreamOfflineV1(Payload {
					message: Message::Notification(data),
					..
				}) => self
					.tx
					.send(InternalMessage::StreamStop {
						channel: data.broadcaster_user_login.to_string(),
					})
					.expect("failed to broadcast stream live"),
				Event::ChannelChatMessageV1(Payload {
					message: Message::Notification(d),
					..
				}) => self
					.tx
					.send(InternalMessage::Debug {
						info: format!("{}: {}", d.chatter_user_name, d.message.text),
					})
					.unwrap_or_else(|_o| {
						println!("failed to broadcast unhandled message");
						return 0;
					}),
				_ => self
					.tx
					.send(InternalMessage::DontHandle)
					.unwrap_or_else(|_o| {
						println!("failed to broadcast unhandled message");
						return 0;
					}),
			},

			EventsubWebsocketData::Welcome { payload, .. } => self
				.tx
				.send(InternalMessage::Init {
					session: payload.session.id.to_string(),
				})
				.expect("Failed to broadcast welcome message"),
			EventsubWebsocketData::Reconnect { payload, .. } => self
				.tx
				.send(InternalMessage::Reconnect {
					session: payload.session.id.into(),
					url: payload
						.session
						.reconnect_url
						.expect("failed to receiv good reconnect url")
						.into(),
				})
				.expect("failed to broadcast reconnect message"),

			_ => self
				.tx
				.send(InternalMessage::DontHandle)
				.unwrap_or_else(|_o| {
					println!("failed to broadcast unhandled message");
					return 0;
				}),
		};

		Ok(())
	}

	async fn on_binary(&mut self, _bytes: ezsockets::Bytes) -> Result<(), ezsockets::Error> {
		self
			.tx
			.send(InternalMessage::DontHandle)
			.expect("ws failed to broadcast on binary");
		Ok(())
	}

	async fn on_call(&mut self, call: Self::Call) -> Result<(), ezsockets::Error> {
		let () = call;
		Ok(())
	}
}
