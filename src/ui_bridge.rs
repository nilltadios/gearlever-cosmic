// SPDX-License-Identifier: GPL-3.0-or-later
//! GUI-owned desktop IPC. The connection dies when the window process exits.
use crate::app::Message;
use cosmic::iced::{self, futures::SinkExt};
use gearlever_cosmic::ui_protocol::{self, Envelope};
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub struct Pending {
    pub envelope: Envelope,
    reply: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}
impl Pending {
    pub fn respond(&self, value: Value) {
        if let Ok(mut reply) = self.reply.lock()
            && let Some(reply) = reply.take()
        {
            let _ = reply.send(value.to_string());
        }
    }
}
struct Bridge {
    requests: mpsc::Sender<Pending>,
}
#[zbus::interface(name = "dev.nilltadios.GearLeverCosmic.UI")]
impl Bridge {
    async fn execute(&self, input: &str) -> String {
        let envelope = match serde_json::from_str::<Envelope>(input) {
            Ok(value) => value,
            Err(error) => return ui_protocol::error(error).to_string(),
        };
        let (sender, receiver) = oneshot::channel();
        let pending = Pending {
            envelope,
            reply: Arc::new(Mutex::new(Some(sender))),
        };
        if self.requests.send(pending).await.is_err() {
            return ui_protocol::error("GUI event loop is unavailable").to_string();
        }
        match tokio::time::timeout(Duration::from_secs(20), receiver).await {
            Ok(Ok(value)) => value,
            _ => ui_protocol::error("GUI command timed out or the window closed").to_string(),
        }
    }
}

pub fn subscription(instance: &str) -> iced::Subscription<Message> {
    iced::Subscription::run_with(Arc::<str>::from(instance), stream)
}
fn stream(instance: &Arc<str>) -> impl iced::futures::Stream<Item = Message> + use<> {
    let instance = instance.to_string();
    iced::stream::channel::<Message>(32, async move |mut output| {
        let (sender, mut receiver) = mpsc::channel(32);
        let connection = async {
            zbus::connection::Builder::session()
                .name(instance)
                .serve_at(ui_protocol::PATH, Bridge { requests: sender })
                .build()
                .await
        }
        .await;
        let _connection = match connection {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("GUI automation IPC unavailable: {error}");
                return;
            }
        };
        while let Some(request) = receiver.recv().await {
            if output.send(Message::UiRequest(request)).await.is_err() {
                break;
            }
        }
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Node {
    pub id: Option<String>,
    pub text: Option<String>,
    pub bounds: [f32; 4],
}
pub struct Tree {
    pub target: Option<iced::advanced::widget::Id>,
    pub nodes: Vec<Node>,
}
impl Tree {
    fn record(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        text: Option<&str>,
    ) {
        if self
            .target
            .as_ref()
            .is_some_and(|target| Some(target) != id)
        {
            return;
        }
        self.nodes.push(Node {
            id: id.map(|id| {
                let name = id.to_string();
                if name == "Undefined" || name == "Set" {
                    format!("{id:?}")
                } else {
                    name
                }
            }),
            text: text.map(str::to_string),
            bounds: [bounds.x, bounds.y, bounds.width, bounds.height],
        });
    }
}
impl iced::advanced::widget::Operation<Vec<Node>> for Tree {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation<Vec<Node>>),
    ) {
        operate(self);
    }
    fn container(&mut self, id: Option<&iced::advanced::widget::Id>, bounds: iced::Rectangle) {
        if id.is_some() {
            self.record(id, bounds, None);
        }
    }
    fn focusable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        _: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        self.record(id, bounds, None);
    }
    fn text_input(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        _: &mut dyn iced::advanced::widget::operation::TextInput,
    ) {
        self.record(id, bounds, None);
    }
    fn text(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        text: &str,
    ) {
        self.record(id, bounds, Some(text));
    }
    fn finish(&self) -> iced::advanced::widget::operation::Outcome<Vec<Node>> {
        iced::advanced::widget::operation::Outcome::Some(self.nodes.clone())
    }
}
