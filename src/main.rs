use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use serde::{Deserialize, Serialize};
use clap::{Parser, Subcommand};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Publish { topic: String, payload: String },
    Hello { message: String },
}

pub struct IPCServer {
    peers: Arc<Mutex<HashMap<String, TcpStream>>>,
}

impl IPCServer {
    pub fn new() -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(&self, port: u16) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", port))?;
        println!("🚀 IPC Server listening on port {}", port);

        let peers = self.peers.clone();

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let peer_addr = stream.peer_addr()?;
                    println!("📡 New peer connected: {}", peer_addr);
                    
                    let peers = peers.clone();
                    thread::spawn(move || {
                        if let Err(e) = Self::handle_peer(stream, peer_addr, peers) {
                            eprintln!("❌ Error handling peer {}: {}", peer_addr, e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("❌ Failed to accept connection: {}", e);
                }
            }
        }

        Ok(())
    }

    fn handle_peer(
        mut stream: TcpStream,
        peer_addr: std::net::SocketAddr,
        peers: Arc<Mutex<HashMap<String, TcpStream>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let peer_id = peer_addr.to_string();
        
        // Add peer to registry
        {
            let mut peers_guard = peers.lock().unwrap();
            peers_guard.insert(peer_id.clone(), stream.try_clone()?);
        }

        let mut buffer = [0; 1024];
        loop {
            let n = stream.read(&mut buffer)?;
            if n == 0 {
                println!("🔌 Peer {} disconnected", peer_addr);
                break;
            }

            let msg_data = &buffer[..n];
            if let Ok(msg) = bincode::deserialize::<Message>(msg_data) {
                println!("📨 Received from {}: {:?}", peer_addr, msg);
                
                match &msg {
                    Message::Publish { topic, payload } => {
                        println!("📢 Publishing to topic '{}': {}", topic, payload);
                        // Broadcast to all peers
                        Self::broadcast_to_peers(&peers, &msg, &peer_id)?;
                    }
                    Message::Hello { message } => {
                        println!("👋 Hello from {}: {}", peer_addr, message);
                        // Echo hello back
                        let hello_response = Message::Hello { 
                            message: format!("Hello back from server!") 
                        };
                        Self::send_message(&mut stream, &hello_response)?;
                    }
                    _ => {
                        // Forward other messages
                        Self::broadcast_to_peers(&peers, &msg, &peer_id)?;
                    }
                }
            }
        }

        // Clean up peer
        {
            let mut peers_guard = peers.lock().unwrap();
            peers_guard.remove(&peer_id);
        }

        Ok(())
    }

    fn broadcast_to_peers(
        peers: &Arc<Mutex<HashMap<String, TcpStream>>>,
        msg: &Message,
        sender_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let peers_guard = peers.lock().unwrap();
        for (peer_id, stream) in peers_guard.iter() {
            if peer_id != sender_id {
                if let Err(e) = Self::send_message(&mut stream.try_clone()?, msg) {
                    eprintln!("❌ Failed to send message to {}: {}", peer_id, e);
                }
            }
        }
        Ok(())
    }

    fn send_message(stream: &mut TcpStream, msg: &Message) -> Result<(), Box<dyn std::error::Error>> {
        let serialized = bincode::serialize(msg)?;
        let len = serialized.len() as u32;
        
        // Send length prefix
        stream.write_all(&len.to_le_bytes())?;
        // Send message
        stream.write_all(&serialized)?;
        stream.flush()?;
        
        Ok(())
    }
}

pub struct IPCClient {
    server_addr: String,
    subscribed_topics: Arc<Mutex<Vec<String>>>,
}

impl IPCClient {
    pub fn new(server_addr: String) -> Self {
        Self {
            server_addr,
            subscribed_topics: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn connect(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut stream = TcpStream::connect(&self.server_addr)?;
        println!("🔗 Connected to IPC server at {}", self.server_addr);

        // Spawn thread to handle incoming messages
        let subscribed_topics = self.subscribed_topics.clone();
        let mut read_stream = stream.try_clone()?;
        thread::spawn(move || {
            let _buffer = [0; 1024];
            loop {
                // Read length prefix
                let mut len_bytes = [0; 4];
                if read_stream.read_exact(&mut len_bytes).is_err() {
                    break;
                }
                let len = u32::from_le_bytes(len_bytes) as usize;
                
                // Read message
                let mut msg_buffer = vec![0; len];
                if read_stream.read_exact(&mut msg_buffer).is_err() {
                    break;
                }
                
                if let Ok(msg) = bincode::deserialize::<Message>(&msg_buffer) {
                    match &msg {
                        Message::Publish { topic, payload } => {
                            let topics = subscribed_topics.lock().unwrap();
                            if topics.contains(topic) {
                                println!("📬 Message on topic '{}': {}", topic, payload);
                            }
                        }
                        Message::Hello { message } => {
                            println!("👋 Server says: {}", message);
                        }
                        _ => {}
                    }
                }
            }
        });

        // Send hello message
        let hello = Message::Hello { 
            message: "Hello from client!".to_string() 
        };
        self.send_message(&mut stream, &hello)?;

        // Keep connection alive and handle user input
        self.handle_user_input(&mut stream)?;

        Ok(())
    }

    fn send_message(&self, stream: &mut TcpStream, msg: &Message) -> Result<(), Box<dyn std::error::Error>> {
        let serialized = bincode::serialize(msg)?;
        let len = serialized.len() as u32;
        
        // Send length prefix
        stream.write_all(&len.to_le_bytes())?;
        // Send message
        stream.write_all(&serialized)?;
        stream.flush()?;
        
        Ok(())
    }

    fn handle_user_input(&self, stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
        use std::io::{self, BufRead};
        
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();

        println!("💬 Enter commands (subscribe <topic>, publish <topic> <message>, or 'quit'):");
        
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let input = line.trim();

            if input == "quit" {
                break;
            }

            let parts: Vec<&str> = input.split_whitespace().collect();
            match parts.as_slice() {
                ["subscribe", topic] => {
                    let msg = Message::Subscribe { topic: topic.to_string() };
                    self.send_message(stream, &msg)?;
                    
                    let mut topics = self.subscribed_topics.lock().unwrap();
                    if !topics.contains(&topic.to_string()) {
                        topics.push(topic.to_string());
                        println!("✅ Subscribed to topic: {}", topic);
                    }
                }
                ["publish", topic, payload @ ..] => {
                    let payload = payload.join(" ");
                    let msg = Message::Publish { 
                        topic: topic.to_string(), 
                        payload: payload.to_string() 
                    };
                    self.send_message(stream, &msg)?;
                    println!("📤 Published to topic '{}': {}", topic, payload);
                }
                _ => {
                    println!("❓ Unknown command. Use: subscribe <topic>, publish <topic> <message>, or quit");
                }
            }
        }

        Ok(())
    }
}

#[derive(Parser)]
#[command(name = "leyline")]
#[command(about = "Pure Rust IPC with TCP/IP pub/sub")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start IPC server
    Server {
        /// Port to listen on
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
    /// Connect as IPC client
    Client {
        /// Server address to connect to
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        server: String,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Server { port } => {
            let server = IPCServer::new();
            server.start(port)?;
        }
        Commands::Client { server } => {
            let client = IPCClient::new(server);
            client.connect()?;
        }
    }

    Ok(())
}