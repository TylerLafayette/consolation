use std::{
    io::{self, BufRead, BufReader, Write},
    net::{TcpStream, ToSocketAddrs},
};

/// A builder which is used to configure and initialize an [`Irc`] connection.
///
/// ## Example
/// ```rust,norun
/// let mut irc = IrcBuilder::default()
///     .with_nickname("nickname")
///     .with_password("my_password")
///     .with_capability("twitch.tv/tags")
///     .with_capability("twitch.tv/members")
///     .connect("irc.chat.twitch.tv:6667")?;
///
/// irc.join("channel")?;
///
/// while let Some(message) = irc.receive()? {
///     println!("message received: {:?}", message);
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct IrcBuilder {
    password: Option<String>,
    nickname: Option<String>,
    capabilities: Vec<String>,
}

impl IrcBuilder {
    /// Specifies a password to authenticate with.
    ///
    /// If specified, a `PASS` message will be sent when [`IrcBuilder::connect`] is called.
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());

        self
    }

    /// Specifies a nickname to use for the IRC session.
    ///
    /// If specified, a `NICK` message will be sent when [`IrcBuilder::connect`] is called.
    pub fn with_nickname(mut self, nickname: impl Into<String>) -> Self {
        self.nickname = Some(nickname.into());

        self
    }

    /// Appends a capability which will be requested from the server on connect. This function can
    /// be called multiple times to request multiple capabilities.
    ///
    /// If one or more capability is added to the builder, a `CAP REQ` message will be sent when
    /// [`IrcBuilder::connect`] is called.
    pub fn with_capability(mut self, capability_name: impl Into<String>) -> Self {
        self.capabilities.push(capability_name.into());

        self
    }

    /// Attempts to connect to the IRC server, returning an [`Irc`] connection handle on success.
    ///
    /// If credentials were previously added to the builder, authorization commands will be sent
    /// when this function is called. Likewise, if capabilities were added, they will be requested
    /// during this function call as well.
    ///
    /// Do not include `irc://` in the `addr` parameter.
    pub fn connect(self, addr: impl ToSocketAddrs) -> io::Result<Irc> {
        let conn = TcpStream::connect(addr)?;
        let reader = BufReader::new(conn.try_clone()?);

        let mut irc = Irc { conn, reader };
        if self.capabilities.len() > 0 {
            irc.request_capabilities(&self.capabilities)?;
        }
        if self.password.is_some() || self.nickname.is_some() {
            irc.authenticate(self.password, self.nickname)?;
        }

        Ok(irc)
    }
}

/// Represents a handle to an open IRC session/connection.
///
/// In order to connect to an IRC server (and construct an [`Irc`]), use an [`IrcBuilder`].
#[derive(Debug)]
pub struct Irc {
    conn: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Irc {
    /// Requests a list of capabilities from the server.
    fn request_capabilities(&mut self, capabilities: &[String]) -> io::Result<()> {
        let capabilities_str = capabilities.join(" ");
        writeln!(self.conn, "CAP REQ :{}", capabilities_str)?;

        Ok(())
    }

    /// Authenticates the user with an optional password and nickname.
    fn authenticate(
        &mut self,
        password: Option<String>,
        nickname: Option<String>,
    ) -> io::Result<()> {
        if let Some(password) = password {
            writeln!(self.conn, "PASS {}", password)?;
        }

        if let Some(nickname) = nickname {
            writeln!(self.conn, "NICK {}", nickname)?;
        }

        Ok(())
    }

    /// Blocks the current thread until the next parseable message is received from the IRC server.
    ///
    /// A value of `Ok(None)` will be returned if and only if the connection is closed.
    pub fn receive(&mut self) -> io::Result<Option<Message>> {
        loop {
            let mut buf = String::new();
            let n = self.reader.read_line(&mut buf)?;
            if n == 0 {
                return Ok(None);
            } else {
                let raw_msg = IrcMessageRaw::parse(&buf)?;
                let message = Message::from_raw_msg(raw_msg)?;

                if let Some(message) = message {
                    return Ok(Some(message));
                }
            }
        }
    }

    /// Joins an IRC channel.
    ///
    /// Do not include a leading `#` in `channel_name`.
    pub fn join(&mut self, channel_name: impl Into<String>) -> io::Result<()> {
        writeln!(self.conn, "JOIN #{}", channel_name.into())
    }
}

/// Represents a private IRC message sent by a user or bot and received in an IRC channel.
#[derive(Debug, Clone)]
pub struct PrivMsg {
    /// The username of the message sender.
    pub username: String,

    /// The body of the message.
    pub message: String,
}

/// Represents an IRC message or event.
#[derive(Debug, Clone)]
pub enum Message {
    /// A private IRC message sent by a user or bot and received in an IRC channel.
    PrivMsg(PrivMsg),
}

impl Message {
    /// Converts a raw [`IrcMessageRaw`] to a user-friendly [`Message`] if there is a suitable
    /// variant, returning `Ok(None)` otherwise.
    fn from_raw_msg(raw_msg: IrcMessageRaw) -> io::Result<Option<Self>> {
        match raw_msg.command_name.as_str() {
            "PRIVMSG" => {
                let username = if let Some(prefix) = raw_msg.prefix {
                    prefix.split("!").next().unwrap_or("").to_string()
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "PRIVMSG missing prefix",
                    ));
                };

                let message = if let Some(message) = raw_msg.command_params.get(1) {
                    message.clone()
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "PRIVMSG missing prefix",
                    ));
                };

                Ok(Some(Self::PrivMsg(PrivMsg { username, message })))
            }
            _ => Ok(None),
        }
    }
}

/// Represents a raw parsed form of an IRC message.
#[derive(Debug, Clone)]
struct IrcMessageRaw {
    /// A list of key-value pairs of message tags sent by the server.
    tags: Vec<(String, String)>,

    /// The user's full prefix, without any additional parsing.
    ///
    /// e.g. `user!user@channel.tmi.twitch.tv`
    prefix: Option<String>,

    /// The name of the command, which is either an arbitrary-length string of letters or a string
    /// of 3 digits.
    command_name: String,

    /// A list of parameters to the command.
    command_params: Vec<String>,
}

impl IrcMessageRaw {
    /// Parses a raw IRC message.
    pub fn parse(input: &str) -> io::Result<Self> {
        let mut chars = input.chars().peekable();

        let mut tags: Vec<(String, String)> = Vec::new();
        let mut prefix: Option<String> = None;

        let first_char = chars.peek();
        if first_char == Some(&'@') {
            let _ = chars.next();

            let tags_str = (&mut chars)
                .take_while(|c| !c.is_whitespace())
                .collect::<String>();
            let mut tags_chars = tags_str.chars();

            loop {
                let key = (&mut tags_chars)
                    .take_while(|c| *c != '=')
                    .collect::<String>();
                if key.len() == 0 {
                    break;
                }

                let value = (&mut tags_chars)
                    .take_while(|c| *c != ';')
                    .collect::<String>();

                tags.push((key, value));
            }
        }

        let mut chars = chars.skip_while(|c| c.is_whitespace()).peekable();

        let first_char = chars.peek();
        if first_char == Some(&':') {
            let _ = chars.next();
            prefix = Some(
                (&mut chars)
                    .take_while(|c| !c.is_whitespace())
                    .collect::<String>(),
            );
        }

        let mut chars = chars.skip_while(|c| c.is_whitespace()).peekable();

        let command_name = (&mut chars)
            .take_while(|c| !c.is_whitespace())
            .collect::<String>();
        if command_name.len() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "error parsing command name: expected command name, reached end of input",
            ));
        }

        let mut command_params = Vec::new();

        while (&mut chars).peek().is_some() {
            let command_param = if chars.peek() == Some(&':') {
                let _ = chars.next();

                (&mut chars)
                    .take_while(|c| *c != '\r' && *c != '\n')
                    .collect::<String>()
            } else {
                (&mut chars)
                    .take_while(|c| !c.is_whitespace() && *c != '\r' && *c != '\n')
                    .collect::<String>()
            };

            command_params.push(command_param);

            if chars.peek() == Some(&'\r') || chars.peek() == Some(&'\n') {
                let _ = (&mut chars)
                    .take_while(|c| *c == '\r' || *c == '\n')
                    .collect::<String>();
            }
        }

        Ok(Self {
            tags,
            prefix,
            command_name,
            command_params,
        })
    }
}
