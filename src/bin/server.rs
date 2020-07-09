use std::collections::HashMap;
use std::sync::Arc;

use futures::{FutureExt, StreamExt};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use warp::ws::{Message, WebSocket};
use warp::Filter;

/// Our state of currently connected users.
///
/// - Key is their id
/// - Value is a sender of `warp::ws::Message`
type Users = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Result<Message, warp::Error>>>>>;

#[derive(Clone)]
struct State {
    users: Users,
    redis: Arc<redis::Client>,
}

#[derive(Debug, Deserialize, Serialize)]
struct UserMsg {
    user_id: usize,
    msg: String,
}

impl UserMsg {
    fn new(user_id: usize, msg: impl ToString) -> UserMsg {
        UserMsg {
            user_id,
            msg: msg.to_string(),
        }
    }
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let users = Users::default();
    let redis = Arc::new(redis::Client::open("redis://127.0.0.1/").unwrap());

    let state = State {
        users: users.clone(),
        redis: redis.clone(),
    };
    let state = warp::any().map(move || state.clone());

    let chat =
        warp::path("chat")
            .and(warp::ws())
            .and(state)
            .map(|ws: warp::ws::Ws, state: State| {
                // This will call our function if the handshake succeeds.
                ws.on_upgrade(move |socket| user_connected(socket, state))
            });

    let index = warp::path::end().map(|| warp::reply::html(INDEX_HTML));
    let routes = index.or(chat);

    let mut pubsub_conn = redis.get_async_connection().await.unwrap().into_pubsub();
    pubsub_conn.subscribe("chat").await.unwrap();

    tokio::task::spawn(async move {
        let mut pubsub_stream = pubsub_conn.on_message();
        while let Ok(msg) = pubsub_stream
            .next()
            .map(|m| m.unwrap().get_payload::<String>())
            .await
        {
            let user_msg: UserMsg = serde_json::from_str(&msg).unwrap();
            user_message(&user_msg, &users).await;
        }
    });

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}

async fn user_connected(ws: WebSocket, state: State) {
    let users = state.users;
    let redis = state.redis;
    let my_id = {
        let mut conn = redis.get_async_connection().await.unwrap();
        conn.incr("user_id", 1u8).await.unwrap()
    };

    eprintln!("new chat user: {}", my_id);

    let (user_ws_tx, mut user_ws_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
        if let Err(e) = result {
            eprintln!("websocket send error: {}", e);
        }
    }));

    users.write().await.insert(my_id, tx);

    let users2 = users.clone();

    let mut publish_conn = redis.get_async_connection().await.unwrap();
    while let Some(result) = user_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("websocket error(uid={}): {}", my_id, e);
                break;
            }
        };

        // Skip any non-Text messages...
        let msg = if let Ok(s) = msg.to_str() {
            let user_msg = UserMsg::new(my_id, s);
            serde_json::to_string(&user_msg).unwrap()
        } else {
            return;
        };

        let _: () = publish_conn.publish("chat", msg).await.unwrap();
    }

    user_disconnected(my_id, &users2).await;
}

async fn user_message(msg: &UserMsg, users: &Users) {
    let new_msg = format!("<User#{}>: {}", msg.user_id, msg.msg);

    for (&uid, tx) in users.read().await.iter() {
        if msg.user_id != uid {
            if let Err(_disconnected) = tx.send(Ok(Message::text(new_msg.clone()))) {
                // The tx is disconnected, our `user_disconnected` code
                // should be happening in another task, nothing more to
                // do here.
            }
        }
    }
}

async fn user_disconnected(my_id: usize, users: &Users) {
    eprintln!("good bye user: {}", my_id);

    users.write().await.remove(&my_id);
}

static INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
    <head>
        <title>Warp Chat</title>
    </head>
    <body>
        <h1>Warp chat</h1>
        <div id="chat">
            <p><em>Connecting...</em></p>
        </div>
        <input type="text" id="text" />
        <button type="button" id="send">Send</button>
        <script type="text/javascript">
        const chat = document.getElementById('chat');
        const text = document.getElementById('text');
        const uri = 'ws://' + location.host + '/chat';
        const ws = new WebSocket(uri);
        function message(data) {
            const line = document.createElement('p');
            line.innerText = data;
            chat.appendChild(line);
        }
        ws.onopen = function() {
            chat.innerHTML = '<p><em>Connected!</em></p>';
        };
        ws.onmessage = function(msg) {
            message(msg.data);
        };
        ws.onclose = function() {
            chat.getElementsByTagName('em')[0].innerText = 'Disconnected!';
        };
        send.onclick = function() {
            const msg = text.value;
            ws.send(msg);
            text.value = '';
            message('<You>: ' + msg);
        };
        </script>
    </body>
</html>
"#;
