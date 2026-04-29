use log::warn;
use twitch_api::eventsub::{Event, EventsubWebsocketData, Message, Payload};

use crate::{data::InternalMessage, err::Error};

pub struct Client {
	pub tx: tokio::sync::broadcast::Sender<InternalMessage>,
	// pub rx: tokio::sync::broadcast::Receiver<InternalMessage>,
}

impl Client {
	pub fn processFrame(&self, data: tungstenite::Utf8Bytes) -> Result<(), Error> {
		if let Err(detail) = twitch_api::eventsub::Event::parse_websocket(&data) {
			warn!("unexpected socket error {}", detail);
			return Ok(());
		};
		let frame = twitch_api::eventsub::Event::parse_websocket(&data).unwrap();

		match frame {
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
					.send(InternalMessage::Chat {
						msg: format!("{}: {}", d.chatter_user_name, d.message.text),
						channel: d.broadcaster_user_login.to_string(),
					})
					.unwrap_or_else(|_o| {
						println!("failed to broadcast chat message");
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
}
