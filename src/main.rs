use std::io;

use consolation::irc::*;

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <channel name>", args[0]);

        return Ok(());
    }
    let channel_name = &args[1];

    let mut irc = IrcBuilder::default()
        .with_nickname("meownadic")
        .with_password(std::env::var("TWITCH_OAUTH_PASS").unwrap())
        .with_capability("twitch.tv/tags")
        .connect("irc.chat.twitch.tv:6667")?;

    irc.join(channel_name)?;

    while let Some(message) = irc.receive()? {
        match message {
            Message::PrivMsg(PrivMsg { username, message }) => {
                println!("{}: {}", username, message);
            }
        }
    }

    Ok(())
}
