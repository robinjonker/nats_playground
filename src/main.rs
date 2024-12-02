// src/main.rs
mod messages;
mod jetstream;           // Import message definitions

use std::collections::HashMap;
use jetstream::JetStreamService;
use messages::KeyPress;
use std::env;
use std::sync::{Arc, Mutex, MutexGuard};
use anyhow::Result;
use tokio::time::Duration;
use crossterm::event::{self, Event, KeyCode};
use rand::Rng;
use crate::jetstream::{JetStreamConfig};
use crate::messages::KEY_PRESS;  // Cross-platform terminal input handling

const STREAM_NAME: &str = "STREAM_19th";
const QUEUE_GROUP: &str = "keypress_processor19th";  // Shared queue group name
const DURABLE_NAME: &str = "keypress_durable19th"; // Shared durable consumer name


#[tokio::main]
async fn main() -> Result<()> {
    // Initialize NATS JetStream service
    let nats_url = env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    // let nats_url = "nats://nats-service.jetstream-messaging.svc.cluster.local:4222";
    let jetstream_service = Arc::new(JetStreamService::new(JetStreamConfig {
        nats_url,
        stream_name: STREAM_NAME.to_string(),
        durable_name: DURABLE_NAME.to_string(),
        subjects: vec![KEY_PRESS.to_string()],
        ack_wait: 500,
        max_deliver: 10,
        redelivery_delay: 100,
        batch_size: 1,
        batch_timeout: 500,
        heartbeat: 100,
        max_messages: 10000,
        max_bytes: 0,
    }).await?);

    // Create a shared Vec of successfully processed messages
    let processed_messages = Arc::new(Mutex::new(Vec::new()));

    // make sure the stream is created
    let stream = jetstream_service.get_or_create_stream().await?;

    // Simulate multiple processing nodes by creating multiple subscribers
    for node_id in 1..=3 {
        let jetstream_clone = Arc::clone(&jetstream_service);
        let stream_clone = stream.clone();
        let processed_messages_clone = Arc::clone(&processed_messages);

        // Add random delay before starting each node
        // let start_delay = rand::thread_rng().gen_range(0..1000);
        // tokio::time::sleep(Duration::from_millis(start_delay)).await;

        tokio::spawn(async move {
            // Create a unique consumer name for each node
            // let consumer_name = format!("{}_{}", DURABLE_NAME, node_id);

            println!("Starting subscriber node {}", node_id);

            match jetstream_clone.subscribe::<KeyPress, _>(
                stream_clone,
                move |keypress: KeyPress| {
                    println!("{}=> Node {} received key press event: {}", chrono::Local::now().naive_local(), node_id, keypress.key);
                    // std::thread::sleep(Duration::from_millis(rand::thread_rng().gen_range(100..500)));
                    // sleep for 100ms
                    std::thread::sleep(Duration::from_millis(150 * node_id));
                    let mut rng = rand::thread_rng();
                    let success = rng.gen_bool(0.5);

                    if success {
                        // Successfully processed; add to the shared Vec
                        let mut messages = processed_messages_clone.lock().unwrap();
                        messages.push(format!("Node {} processed key press: {}", node_id, keypress.key));
                        println!("{}=> Node {} successfully processed key press event: {}", chrono::Local::now().naive_local(), node_id, keypress.key);
                        Ok(()) // Return Ok to acknowledge
                    } else {
                        // Simulate failure by not acknowledging the message
                        println!("{}=> Node {} failed to process key press event: {}", chrono::Local::now().naive_local(), node_id, keypress.key);
                        Err(()) // Return Err to not acknowledge
                    }
                },
            ).await {
                Ok(_) => println!("Node {} subscribed successfully", node_id),
                Err(e) => eprintln!("Node {} failed to subscribe: {:?}", node_id, e),
            };
        });
    }

    println!("Press the 'space' key to send an event. Press 'q' to quit.");

    // Continuously listen for keyboard input
    loop {
        // Check if an event is available
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key_event) = event::read()? {
                match key_event.code {
                    KeyCode::Char('q') => {
                        println!("Exiting...");
                        break;
                    },
                    KeyCode::Enter => {},
                    _ => {
                        // Publish a KeyPress event for the space key
                        // In main.rs, modify the publish call:
                        let keypress = KeyPress { key: format!("{:?}", key_event.code) };
                        jetstream_service.publish(KEY_PRESS, &keypress).await?;
                    }
                }
            }
        }
    }

    // Print the processed messages summary, grouped by node
    println!("\nProcessed messages summary:");
    let processed_messages = processed_messages.lock().unwrap();
    let grouped_messages = group_messages_by_node(&processed_messages);

    for (node, messages) in grouped_messages {
        println!("Node {} processed: {:?}", node, messages);
    }

    // Helper function to group messages by node
    fn group_messages_by_node(processed_messages: &MutexGuard<Vec<String>>) -> HashMap<String, Vec<String>> {
        let mut grouped: HashMap<String, Vec<String>> = HashMap::new();

        for message in processed_messages.iter() {
            if let Some((node, processed)) = message.split_once("processed key press:") {
                let node = node.trim().to_string();
                let processed = processed.trim().to_string();
                grouped.entry(node).or_default().push(processed);
            }
        }

        grouped
    }

    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}