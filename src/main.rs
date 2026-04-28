use clap::{Parser, Subcommand};
use std::net::{TcpStream, Ipv4Addr};
use std::io::{Write, BufReader};
use uuid::Uuid;
use crate::proto::{
    Encode,
    ConnectionState,
    HandshakeIntent,
    Packet,
    ServerboundPacket,
};

pub mod proto;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[arg(short, long)]
    addr: String,
    #[arg(short, long, default_value_t = 25565)]
    port: u16,
    #[arg(long, default_value_t = 774)]
    proto: i32,
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    Status {},
    Connect {
        #[arg(short, long)]
        username: String,
    },
}

fn main() {
    let args = Cli::parse();
    let _addr: Ipv4Addr = args.addr.parse().expect("Invalid IPv4 address");
    let full_addr = format!("{}:{}", args.addr, args.port);
    println!("Connecting to {}", full_addr);
    let mut stream = TcpStream::connect(&full_addr)
                            .expect("Couldn't connect to the server...");
    println!("Connected");

    let mut compression = -1;
    match args.command {
        // TODO: somehow create common bufreader
        Commands::Status {} => {
            let handshake = ServerboundPacket::Handshake{
                proto_version: args.proto,
                server_address: args.addr,
                server_port: args.port,
                intent: HandshakeIntent::Status,
            };
            let handshake_data = handshake.encode().expect("could not encode handshake data");
            if args.verbose {
                println!("handshake data: {:x?}", handshake_data);
            }
            stream.write(&handshake_data).expect("write error");
            let status_request = ServerboundPacket::StatusRequest;
            let sr_data = status_request.encode().expect("could not encode status request data");
            if args.verbose {
                println!("status request data: {:x?}", sr_data);
            }
            stream.write(&sr_data).expect("write error");
            match Packet::read_from(
                &mut BufReader::new(&stream), ConnectionState::Status, compression,
            ) {
                Ok(p) => match p {
                    Packet::StatusResponse {data} => { println!("{}", data) },
                    _ => unreachable!(),
                },
                Err(e) => panic!("{}", e),
            }
            stream.write(
                &ServerboundPacket::PingRequest
                    .encode()
                    .expect("could not encode ping request")
            ).expect("write error");
            match Packet::read_from(&mut BufReader::new(&stream), ConnectionState::Status, compression) {
                Ok(p) => match p {
                    Packet::PongResponse {start_timestamp, stop_timestamp} => {
                        println!("ping ms: {}", stop_timestamp - start_timestamp);
                    },
                    _ => unreachable!(),
                },
                Err(e) => panic!("{}", e),
            }
        }
        Commands::Connect {username} => {
            let handshake = ServerboundPacket::Handshake{
                proto_version: args.proto,
                server_address: args.addr,
                server_port: args.port,
                intent: HandshakeIntent::Login,
            };
            let handshake_data = handshake.encode().expect("could not encode handshake data");
            if args.verbose {
                println!("handshake data: {:x?}", handshake_data);
            }
            stream.write(&handshake_data).expect("write error");
            let login_start = ServerboundPacket::LoginStart{
                username: &username,
                uuid: Uuid::new_v4(),
            };
            let ls_data = login_start.encode().expect("could not encode login start data");
            if args.verbose {
                println!("login start data: {:x?}", ls_data);
            };
            stream.write(&ls_data).expect("write error");
            let mut r = BufReader::new(&mut stream);
            match Packet::read_from(&mut r, ConnectionState::Login, compression) {
                Ok(p) => match p {
                    Packet::LoginDisconnect {reason} => { panic!("disconnected: {}", reason) },
                    Packet::SetCompression {threshold} => {
                        println!("compression threshold: {}", threshold);
                        compression = threshold;
                    },
                    Packet::LoginSuccess {game_profile} => {
                        println!("username: {}, uuid: {}", game_profile.username, game_profile.uuid)
                    },
                    _ => unreachable!(),
                },
                Err(e) => panic!("{}", e),
            };
            match Packet::read_from(&mut r, ConnectionState::Login, compression) {
                Ok(p) => match p {
                    Packet::LoginDisconnect {reason} => { panic!("disconnected: {}", reason) },
                    Packet::LoginSuccess {game_profile} => {
                        println!("logged in as {} ({})", game_profile.username, game_profile.uuid)
                    },
                    _ => unreachable!(),
                },
                Err(e) => panic!("{}", e),
            }
            stream.write(&ServerboundPacket::LoginAcknowledged.encode()
                .expect("could not encode login ack data"))
                .expect("login ack write error");
        }
    }
}

