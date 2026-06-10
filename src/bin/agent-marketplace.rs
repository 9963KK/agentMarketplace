use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_SERVER: &str = "http://127.0.0.1:8080";
const CREDENTIAL_DIR: &str = ".agent-marketplace";
const CREDENTIAL_FILE: &str = "credentials.json";

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), CliError> {
    let mut parser = ArgParser::new(args);
    if parser.is_empty() {
        print_help();
        return Ok(());
    }

    let global_server = parser.take_option("--server");
    let global_token = parser.take_option("--token");
    let global_agent_id = parser.take_option("--agent-id");
    let command = parser.command()?;
    if command == "help" || command == "--help" || command == "-h" {
        print_help();
        return Ok(());
    }
    let credentials = Credentials::load().unwrap_or_default();
    let server = global_server
        .or_else(|| env::var("AGENT_MARKETPLACE_SERVER").ok())
        .or_else(|| credentials.server.clone())
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());
    let token = global_token
        .or_else(|| env::var("AGENT_MARKETPLACE_TOKEN").ok())
        .or_else(|| credentials.token.clone());
    let agent_id = global_agent_id
        .or_else(|| env::var("AGENT_MARKETPLACE_AGENT_ID").ok())
        .or_else(|| credentials.agent_id.clone());
    let client = HttpClient::new(server.clone())?;

    match command.as_str() {
        "register" => {
            let agent_id = agent_id
                .or_else(|| parser.take_option("--id"))
                .ok_or_else(|| CliError::usage("register requires --agent-id"))?;
            let name = parser.take_option("--name");
            let endpoint = parser.take_option("--endpoint");
            let metadata = parser.take_kv_options("--metadata")?;
            parser.finish()?;

            let response = client.post_json(
                "/agents/register",
                None,
                None,
                &json!({
                    "agent_id": agent_id,
                    "name": name,
                    "endpoint": endpoint,
                    "metadata": metadata,
                }),
            )?;
            if let Some(credentials) = credentials_from_register_response(&server, &response) {
                credentials.save()?;
            }
            print_json(&response)?;
        }
        "declare-capabilities" => {
            let token = require_token(token)?;
            let capabilities = parser
                .take_option("--capabilities")
                .ok_or_else(|| CliError::usage("declare-capabilities requires --capabilities"))?;
            let max_concurrency = parser
                .take_option("--max-concurrency")
                .map(|value| parse_u32("--max-concurrency", &value))
                .transpose()?
                .unwrap_or(1);
            parser.finish()?;

            let capabilities = capabilities
                .split(',')
                .filter(|value| !value.trim().is_empty())
                .map(|name| {
                    json!({
                        "name": name.trim(),
                        "max_concurrency": max_concurrency,
                        "contract": null,
                    })
                })
                .collect::<Vec<_>>();
            client.put_json(
                "/agents/capabilities",
                Some(&token),
                None,
                &json!({ "capabilities": capabilities }),
            )?;
            println!("ok");
        }
        "ping" => {
            let token = require_token(token)?;
            let busy = parser.take_flag("--busy");
            parser.finish()?;

            let response = client.post_json(
                "/agents/heartbeat",
                Some(&token),
                None,
                &json!({ "busy": busy }),
            )?;
            print_json(&response)?;
        }
        "daemon" => {
            let token = require_token(token)?;
            let interval = parser
                .take_option("--interval")
                .map(|value| parse_u64("--interval", &value))
                .transpose()?
                .unwrap_or(5);
            let busy = parser.take_flag("--busy");
            parser.finish()?;

            loop {
                let response = client.post_json(
                    "/agents/heartbeat",
                    Some(&token),
                    None,
                    &json!({ "busy": busy }),
                )?;
                println!("{}", compact_json(&response)?);
                thread::sleep(Duration::from_secs(interval));
            }
        }
        "discover" => {
            let capability = parser
                .take_option("--capability")
                .or_else(|| parser.take_option("--cap"))
                .ok_or_else(|| CliError::usage("discover requires --capability"))?;
            let include_busy = parser.take_flag("--include-busy");
            let limit = parser.take_option("--limit");
            parser.finish()?;

            let mut path = format!("/agents/discover?cap={}", encode_query(&capability));
            if include_busy {
                path.push_str("&include_busy=true");
            }
            if let Some(limit) = limit {
                path.push_str("&limit=");
                path.push_str(&encode_query(&limit));
            }
            let response = client.get(&path, None)?;
            print_json(&response)?;
        }
        "create-task" => {
            let token = require_token(token)?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("create-task"));
            parser.finish()?;

            let response = client.post_empty("/tasks", Some(&token), Some(&idempotency_key))?;
            print_json(&response)?;
        }
        "add-participant" => {
            let token = require_token(token)?;
            let task_id = parser
                .take_option("--task-id")
                .ok_or_else(|| CliError::usage("add-participant requires --task-id"))?;
            let participant = parser
                .take_option("--participant-agent-id")
                .or_else(|| parser.take_option("--participant"))
                .ok_or_else(|| {
                    CliError::usage("add-participant requires --participant-agent-id")
                })?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("add-participant"));
            parser.finish()?;

            client.post_json(
                &format!("/tasks/{task_id}/participants"),
                Some(&token),
                Some(&idempotency_key),
                &json!({ "agent_id": participant }),
            )?;
            println!("ok");
        }
        "create-session" => {
            let token = require_token(token)?;
            let task_id = parser
                .take_option("--task-id")
                .ok_or_else(|| CliError::usage("create-session requires --task-id"))?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("create-session"));
            parser.finish()?;

            let response = client.post_json(
                "/sessions",
                Some(&token),
                Some(&idempotency_key),
                &json!({ "task_id": task_id }),
            )?;
            print_json(&response)?;
        }
        "assign" => {
            let token = require_token(token)?;
            let task_id = parser
                .take_option("--task-id")
                .ok_or_else(|| CliError::usage("assign requires --task-id"))?;
            let session_id = parser
                .take_option("--session-id")
                .ok_or_else(|| CliError::usage("assign requires --session-id"))?;
            let assignee = parser
                .take_option("--assignee-agent-id")
                .or_else(|| parser.take_option("--assignee"))
                .ok_or_else(|| CliError::usage("assign requires --assignee-agent-id"))?;
            let kind = parser
                .take_option("--kind")
                .ok_or_else(|| CliError::usage("assign requires --kind execute|review"))?;
            let target_assignment_id = parser.take_option("--target-assignment-id");
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("assign"));
            parser.finish()?;

            let response = client.post_json(
                "/assignments",
                Some(&token),
                Some(&idempotency_key),
                &json!({
                    "task_id": task_id,
                    "session_id": session_id,
                    "agent_id": assignee,
                    "kind": assignment_kind_json(&kind, target_assignment_id)?,
                }),
            )?;
            print_json(&response)?;
        }
        "get-assignment" => {
            let assignment_id = parser
                .take_option("--assignment-id")
                .ok_or_else(|| CliError::usage("get-assignment requires --assignment-id"))?;
            parser.finish()?;

            let response = client.get(&format!("/assignments/{assignment_id}"), None)?;
            print_json(&response)?;
        }
        "review-assignments-for-target" => {
            let assignment_id = parser.take_option("--assignment-id").ok_or_else(|| {
                CliError::usage("review-assignments-for-target requires --assignment-id")
            })?;
            parser.finish()?;

            let response = client.get(
                &format!("/assignments/{assignment_id}/review-assignments"),
                None,
            )?;
            print_json(&response)?;
        }
        "submit-artifact" => {
            let token = require_token(token)?;
            let assignment_id = parser
                .take_option("--assignment-id")
                .ok_or_else(|| CliError::usage("submit-artifact requires --assignment-id"))?;
            let manifest = parser
                .take_option("--manifest")
                .ok_or_else(|| CliError::usage("submit-artifact requires --manifest"))?;
            let manifest_uri = parser
                .take_option("--manifest-uri")
                .ok_or_else(|| CliError::usage("submit-artifact requires --manifest-uri"))?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("submit-artifact"));
            parser.finish()?;

            let response = client.put_json(
                &format!("/assignments/{assignment_id}/artifact"),
                Some(&token),
                Some(&idempotency_key),
                &json!({
                    "manifest": read_json_file(&manifest)?,
                    "manifest_uri": manifest_uri,
                }),
            )?;
            print_json(&response)?;
        }
        "get-artifact-locator" => {
            let assignment_id = parser
                .take_option("--assignment-id")
                .ok_or_else(|| CliError::usage("get-artifact-locator requires --assignment-id"))?;
            parser.finish()?;

            let response = client.get(
                &format!("/assignments/{assignment_id}/artifact-locator"),
                None,
            )?;
            print_json(&response)?;
        }
        "request-review" => {
            let token = require_token(token)?;
            let task_id = parser
                .take_option("--task-id")
                .ok_or_else(|| CliError::usage("request-review requires --task-id"))?;
            let target_assignment_id = parser
                .take_option("--target-assignment-id")
                .ok_or_else(|| CliError::usage("request-review requires --target-assignment-id"))?;
            let review_assignment_ids = parser
                .take_option("--review-assignment-ids")
                .ok_or_else(|| CliError::usage("request-review requires --review-assignment-ids"))?
                .split(',')
                .filter(|value| !value.trim().is_empty())
                .map(|value| Value::String(value.trim().to_string()))
                .collect::<Vec<_>>();
            let criteria = review_criteria_json(&mut parser)?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("request-review"));
            parser.finish()?;

            let response = client.post_json(
                "/reviews",
                Some(&token),
                Some(&idempotency_key),
                &json!({
                    "task_id": task_id,
                    "target_assignment_id": target_assignment_id,
                    "review_assignment_ids": review_assignment_ids,
                    "criteria": criteria,
                }),
            )?;
            print_json(&response)?;
        }
        "reviews-by-assignment" => {
            let assignment_id = parser
                .take_option("--assignment-id")
                .ok_or_else(|| CliError::usage("reviews-by-assignment requires --assignment-id"))?;
            parser.finish()?;

            let response = client.get(&format!("/reviews/by-assignment/{assignment_id}"), None)?;
            print_json(&response)?;
        }
        "submit-review" => {
            let token = require_token(token)?;
            let review_id = parser
                .take_option("--review-id")
                .ok_or_else(|| CliError::usage("submit-review requires --review-id"))?;
            let review_assignment_id = parser
                .take_option("--review-assignment-id")
                .ok_or_else(|| CliError::usage("submit-review requires --review-assignment-id"))?;
            let verdict = parser
                .take_option("--verdict")
                .ok_or_else(|| CliError::usage("submit-review requires --verdict"))?;
            let score_bps = parser
                .take_option("--score-bps")
                .map(|value| parse_u16("--score-bps", &value))
                .transpose()?
                .unwrap_or(10_000);
            let feedback = parser.take_option("--feedback").unwrap_or_default();
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("submit-review"));
            parser.finish()?;

            client.post_json(
                &format!("/reviews/{review_id}/verdict"),
                Some(&token),
                Some(&idempotency_key),
                &json!({
                    "review_assignment_id": review_assignment_id,
                    "verdict": {
                        "kind": verdict_kind_json(&verdict)?,
                        "score_bps": score_bps,
                        "feedback": feedback,
                    }
                }),
            )?;
            println!("ok");
        }
        "my-assignments" => {
            let token = require_token(token)?;
            let agent_id = agent_id.ok_or_else(|| {
                CliError::usage("my-assignments requires --agent-id or saved credentials")
            })?;
            parser.finish()?;

            let response = client.get(&format!("/agents/{agent_id}/assignments"), Some(&token))?;
            print_json(&response)?;
        }
        "deposit" => {
            let token = require_token(token)?;
            let amount = parser
                .take_option("--amount")
                .ok_or_else(|| CliError::usage("deposit requires --amount"))
                .and_then(|value| parse_u64("--amount", &value))?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("deposit"));
            parser.finish()?;

            client.post_json(
                "/settlement/deposit",
                Some(&token),
                Some(&idempotency_key),
                &json!({ "amount": amount }),
            )?;
            println!("ok");
        }
        "hold" => {
            let token = require_token(token)?;
            let from_agent = agent_id
                .or_else(|| parser.take_option("--from-agent"))
                .ok_or_else(|| CliError::usage("hold requires --agent-id or --from-agent"))?;
            let amount = parser
                .take_option("--amount")
                .ok_or_else(|| CliError::usage("hold requires --amount"))
                .and_then(|value| parse_u64("--amount", &value))?;
            let task_id = parser
                .take_option("--task-id")
                .ok_or_else(|| CliError::usage("hold requires --task-id"))?;
            let assignment_id = parser
                .take_option("--assignment-id")
                .ok_or_else(|| CliError::usage("hold requires --assignment-id"))?;
            let payee = parser
                .take_option("--payee-agent-id")
                .or_else(|| parser.take_option("--payee"))
                .ok_or_else(|| CliError::usage("hold requires --payee-agent-id"))?;
            let kind = parser
                .take_option("--kind")
                .ok_or_else(|| CliError::usage("hold requires --kind execute|review"))?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("hold"));
            parser.finish()?;

            let response = client.post_json(
                "/settlement/hold",
                Some(&token),
                Some(&idempotency_key),
                &json!({
                    "from_agent": from_agent,
                    "amount": amount,
                    "task_id": task_id,
                    "assignment_id": assignment_id,
                    "agent_id": payee,
                    "kind": hold_kind_json(&kind)?,
                }),
            )?;
            print_json(&response)?;
        }
        "refund" => {
            let token = require_token(token)?;
            let hold_id = parser
                .take_option("--hold-id")
                .ok_or_else(|| CliError::usage("refund requires --hold-id"))?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("refund"));
            parser.finish()?;

            client.post_json(
                "/settlement/refund",
                Some(&token),
                Some(&idempotency_key),
                &json!({ "hold_id": hold_id }),
            )?;
            println!("ok");
        }
        "settle-execute" => {
            let token = require_token(token)?;
            let hold_id = parser
                .take_option("--hold-id")
                .ok_or_else(|| CliError::usage("settle-execute requires --hold-id"))?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("settle-execute"));
            parser.finish()?;

            client.post_json(
                "/settlement/release-execute-after-reviews",
                Some(&token),
                Some(&idempotency_key),
                &json!({ "hold_id": hold_id }),
            )?;
            println!("ok");
        }
        "settle-review" => {
            let token = require_token(token)?;
            let hold_id = parser
                .take_option("--hold-id")
                .ok_or_else(|| CliError::usage("settle-review requires --hold-id"))?;
            let review_id = parser
                .take_option("--review-id")
                .ok_or_else(|| CliError::usage("settle-review requires --review-id"))?;
            let idempotency_key = parser
                .take_option("--idempotency-key")
                .unwrap_or_else(|| idempotency_key("settle-review"));
            parser.finish()?;

            client.post_json(
                "/settlement/release-review-after-submission",
                Some(&token),
                Some(&idempotency_key),
                &json!({ "hold_id": hold_id, "review_id": review_id }),
            )?;
            println!("ok");
        }
        "balance" => {
            let token = require_token(token)?;
            parser.finish()?;

            let response = client.get("/settlement/balance", Some(&token))?;
            print_json(&response)?;
        }
        "deregister" => {
            let token = require_token(token)?;
            parser.finish()?;

            let response = client.post_empty("/agents/deregister", Some(&token), None)?;
            Credentials::remove().ok();
            print_json(&response)?;
        }
        _ => return Err(CliError::usage(format!("unknown command: {command}"))),
    }

    Ok(())
}

fn require_token(token: Option<String>) -> Result<String, CliError> {
    token.ok_or_else(|| CliError::usage("missing token; use --token or register first"))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Credentials {
    server: Option<String>,
    agent_id: Option<String>,
    token: Option<String>,
}

impl Credentials {
    fn load() -> Result<Self, CliError> {
        let path = credential_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }

    fn save(&self) -> Result<(), CliError> {
        let path = credential_path()?;
        let parent = path
            .parent()
            .ok_or_else(|| CliError::message("invalid credential path"))?;
        fs::create_dir_all(parent)?;
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    fn remove() -> Result<(), CliError> {
        let path = credential_path()?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn credential_path() -> Result<PathBuf, CliError> {
    let home = env::var("HOME").map_err(|_| CliError::message("HOME is not set"))?;
    Ok(PathBuf::from(home)
        .join(CREDENTIAL_DIR)
        .join(CREDENTIAL_FILE))
}

fn credentials_from_register_response(server: &str, response: &Value) -> Option<Credentials> {
    Some(Credentials {
        server: Some(server.to_string()),
        agent_id: Some(response.get("agent_id")?.as_str()?.to_string()),
        token: Some(response.get("token")?.as_str()?.to_string()),
    })
}

#[derive(Clone, Debug)]
struct HttpClient {
    base: HttpBase,
}

impl HttpClient {
    fn new(server: String) -> Result<Self, CliError> {
        Ok(Self {
            base: HttpBase::parse(&server)?,
        })
    }

    fn get(&self, path: &str, token: Option<&str>) -> Result<Value, CliError> {
        self.request("GET", path, token, None, None)
    }

    fn post_empty(
        &self,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, CliError> {
        self.request("POST", path, token, idempotency_key, None)
    }

    fn post_json(
        &self,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
        body: &Value,
    ) -> Result<Value, CliError> {
        self.request("POST", path, token, idempotency_key, Some(body))
    }

    fn put_json(
        &self,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
        body: &Value,
    ) -> Result<Value, CliError> {
        self.request("PUT", path, token, idempotency_key, Some(body))
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value, CliError> {
        let body = body
            .map(serde_json::to_vec)
            .transpose()?
            .unwrap_or_default();
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.base.host_header()
        );
        if let Some(token) = token {
            request.push_str("Authorization: Bearer ");
            request.push_str(token);
            request.push_str("\r\n");
        }
        if let Some(key) = idempotency_key {
            request.push_str("Idempotency-Key: ");
            request.push_str(key);
            request.push_str("\r\n");
        }
        if !body.is_empty() {
            request.push_str("Content-Type: application/json\r\n");
        }
        request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));

        let mut stream = TcpStream::connect((self.base.host.as_str(), self.base.port))?;
        stream.write_all(request.as_bytes())?;
        if !body.is_empty() {
            stream.write_all(&body)?;
        }
        stream.flush()?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        parse_http_response(&response)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HttpBase {
    host: String,
    port: u16,
}

impl HttpBase {
    fn parse(value: &str) -> Result<Self, CliError> {
        let value = value
            .strip_prefix("http://")
            .ok_or_else(|| CliError::usage("only http:// servers are supported"))?;
        let authority = value.split('/').next().unwrap_or(value);
        if authority.is_empty() {
            return Err(CliError::usage("server host is empty"));
        }
        let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
            (host.to_string(), parse_u16("server port", port)?)
        } else {
            (authority.to_string(), 80)
        };
        Ok(Self { host, port })
    }

    fn host_header(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

fn parse_http_response(response: &[u8]) -> Result<Value, CliError> {
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| CliError::message("invalid HTTP response"))?;
    let headers = std::str::from_utf8(&response[..split])
        .map_err(|_| CliError::message("HTTP response headers are not UTF-8"))?;
    let mut lines = headers.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| CliError::message("missing HTTP status line"))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| CliError::message("missing HTTP status code"))
        .and_then(|value| parse_u16("HTTP status", value))?;
    let body = &response[split + 4..];
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).trim().to_string()))
    };
    if !(200..300).contains(&status) {
        return Err(CliError::Http {
            status,
            body: value,
        });
    }
    Ok(value)
}

#[derive(Debug)]
struct ArgParser {
    args: Vec<String>,
}

impl ArgParser {
    fn new(args: Vec<String>) -> Self {
        Self { args }
    }

    fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    fn command(&mut self) -> Result<String, CliError> {
        if self.args.is_empty() {
            return Err(CliError::usage("missing command"));
        }
        Ok(self.args.remove(0))
    }

    fn take_option(&mut self, name: &str) -> Option<String> {
        let index = self.args.iter().position(|arg| arg == name)?;
        self.args.remove(index);
        if index >= self.args.len() {
            return Some(String::new());
        }
        Some(self.args.remove(index))
    }

    fn take_flag(&mut self, name: &str) -> bool {
        let Some(index) = self.args.iter().position(|arg| arg == name) else {
            return false;
        };
        self.args.remove(index);
        true
    }

    fn take_kv_options(&mut self, name: &str) -> Result<BTreeMap<String, String>, CliError> {
        let mut values = BTreeMap::new();
        while let Some(value) = self.take_option(name) {
            let Some((key, value)) = value.split_once('=') else {
                return Err(CliError::usage(format!("{name} expects key=value")));
            };
            values.insert(key.to_string(), value.to_string());
        }
        Ok(values)
    }

    fn finish(self) -> Result<(), CliError> {
        if self.args.is_empty() {
            return Ok(());
        }
        Err(CliError::usage(format!(
            "unexpected arguments: {}",
            self.args.join(" ")
        )))
    }
}

#[derive(Debug)]
enum CliError {
    Usage(String),
    Message(String),
    Http { status: u16, body: Value },
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage(message) => write!(f, "{message}\n\nRun `agent-marketplace help`."),
            CliError::Message(message) => f.write_str(message),
            CliError::Http { status, body } => {
                write!(
                    f,
                    "HTTP {status}: {}",
                    compact_json(body).unwrap_or_else(|_| body.to_string())
                )
            }
            CliError::Io(error) => write!(f, "{error}"),
            CliError::Json(error) => write!(f, "{error}"),
        }
    }
}

impl Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

fn read_json_file(path: &str) -> Result<Value, CliError> {
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}

fn assignment_kind_json(
    kind: &str,
    target_assignment_id: Option<String>,
) -> Result<Value, CliError> {
    match normalize_kind(kind).as_str() {
        "execute" => Ok(Value::String("Execute".to_string())),
        "review" => {
            let target_assignment_id = target_assignment_id.ok_or_else(|| {
                CliError::usage("review assignment requires --target-assignment-id")
            })?;
            Ok(json!({
                "Review": {
                    "target_assignment_id": target_assignment_id
                }
            }))
        }
        _ => Err(CliError::usage("--kind must be execute or review")),
    }
}

fn hold_kind_json(kind: &str) -> Result<Value, CliError> {
    match normalize_kind(kind).as_str() {
        "execute" => Ok(Value::String("Execute".to_string())),
        "review" => Ok(Value::String("Review".to_string())),
        _ => Err(CliError::usage("--kind must be execute or review")),
    }
}

fn verdict_kind_json(kind: &str) -> Result<Value, CliError> {
    let value = match normalize_kind(kind).as_str() {
        "passed" | "pass" => "Passed",
        "failed" | "fail" => "Failed",
        "artifact-unavailable" | "artifact_unavailable" => "ArtifactUnavailable",
        "hash-mismatch" | "hash_mismatch" => "HashMismatch",
        "invalid-format" | "invalid_format" => "InvalidFormat",
        _ => {
            return Err(CliError::usage(
                "--verdict must be passed, failed, artifact-unavailable, hash-mismatch, or invalid-format",
            ));
        }
    };
    Ok(Value::String(value.to_string()))
}

fn review_criteria_json(parser: &mut ArgParser) -> Result<Value, CliError> {
    if let Some(path) = parser.take_option("--criteria-json") {
        return read_json_file(&path);
    }
    if let Some(path) = parser.take_option("--criteria-file") {
        let body = fs::read_to_string(path)?;
        let format = parser
            .take_option("--criteria-format")
            .unwrap_or_else(|| "PlainText".to_string());
        return Ok(json!({
            "format": criteria_format_json(&format)?,
            "body": body,
        }));
    }
    let body = parser.take_option("--criteria").ok_or_else(|| {
        CliError::usage("request-review requires --criteria, --criteria-file, or --criteria-json")
    })?;
    let format = parser
        .take_option("--criteria-format")
        .unwrap_or_else(|| "PlainText".to_string());
    Ok(json!({
        "format": criteria_format_json(&format)?,
        "body": body,
    }))
}

fn criteria_format_json(format: &str) -> Result<Value, CliError> {
    match normalize_kind(format).as_str() {
        "plain-text" | "plaintext" => Ok(Value::String("PlainText".to_string())),
        "json" => Ok(Value::String("Json".to_string())),
        _ => Err(CliError::usage(
            "--criteria-format must be plain-text or json",
        )),
    }
}

fn normalize_kind(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn parse_u16(name: &str, value: &str) -> Result<u16, CliError> {
    value
        .parse::<u16>()
        .map_err(|_| CliError::usage(format!("{name} must be a valid u16")))
}

fn parse_u32(name: &str, value: &str) -> Result<u32, CliError> {
    value
        .parse::<u32>()
        .map_err(|_| CliError::usage(format!("{name} must be a valid u32")))
}

fn parse_u64(name: &str, value: &str) -> Result<u64, CliError> {
    value
        .parse::<u64>()
        .map_err(|_| CliError::usage(format!("{name} must be a valid u64")))
}

fn idempotency_key(operation: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{operation}-{}-{now}", process::id())
}

fn encode_query(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                vec![byte as char]
            }
            b' ' => vec!['+'],
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn print_json(value: &Value) -> Result<(), CliError> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn compact_json(value: &Value) -> Result<String, CliError> {
    Ok(serde_json::to_string(value)?)
}

fn print_help() {
    println!(
        r#"agent-marketplace <command> [options]

Global options:
  --server <url>          Default: AGENT_MARKETPLACE_SERVER or http://127.0.0.1:8080
  --token <token>         Default: AGENT_MARKETPLACE_TOKEN or saved credentials
  --agent-id <id>         Default: AGENT_MARKETPLACE_AGENT_ID or saved credentials

Commands:
  register --agent-id <id> [--name <name>] [--endpoint <url>] [--metadata k=v]
  declare-capabilities --capabilities a,b [--max-concurrency n]
  ping [--busy]
  daemon [--interval seconds] [--busy]
  discover --capability <name> [--include-busy] [--limit n]
  create-task
  add-participant --task-id <id> --participant-agent-id <id>
  create-session --task-id <id>
  assign --task-id <id> --session-id <id> --assignee-agent-id <id> --kind execute
  assign --task-id <id> --session-id <id> --assignee-agent-id <id> --kind review --target-assignment-id <id>
  get-assignment --assignment-id <id>
  my-assignments
  review-assignments-for-target --assignment-id <id>
  submit-artifact --assignment-id <id> --manifest <file.json> --manifest-uri <uri>
  get-artifact-locator --assignment-id <id>
  request-review --task-id <id> --target-assignment-id <id> --review-assignment-ids a,b --criteria <text>
  reviews-by-assignment --assignment-id <id>
  submit-review --review-id <id> --review-assignment-id <id> --verdict passed [--score-bps n] [--feedback text]
  deposit --amount <n>
  hold --amount <n> --task-id <id> --assignment-id <id> --payee-agent-id <id> --kind execute|review
  refund --hold-id <id>
  settle-execute --hold-id <id>
  settle-review --hold-id <id> --review-id <id>
  balance
  deregister
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_base_with_default_and_explicit_port() {
        assert_eq!(
            HttpBase::parse("http://127.0.0.1:8080").unwrap(),
            HttpBase {
                host: "127.0.0.1".to_string(),
                port: 8080
            }
        );
        assert_eq!(
            HttpBase::parse("http://example.com/api").unwrap(),
            HttpBase {
                host: "example.com".to_string(),
                port: 80
            }
        );
    }

    #[test]
    fn parses_successful_http_response_as_json() {
        let value =
            parse_http_response(b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\n\r\n{\"ok\":true}")
                .unwrap();

        assert_eq!(value, json!({ "ok": true }));
    }

    #[test]
    fn rejects_http_error_status() {
        let error = parse_http_response(
            b"HTTP/1.1 400 Bad Request\r\ncontent-length: 15\r\n\r\n{\"error\":\"bad\"}",
        )
        .unwrap_err();

        assert!(matches!(error, CliError::Http { status: 400, .. }));
    }

    #[test]
    fn extracts_repeated_metadata_options() {
        let mut parser = ArgParser::new(vec![
            "--metadata".to_string(),
            "runtime=codex".to_string(),
            "--metadata".to_string(),
            "role=reviewer".to_string(),
        ]);

        let metadata = parser.take_kv_options("--metadata").unwrap();

        assert_eq!(metadata.get("runtime").unwrap(), "codex");
        assert_eq!(metadata.get("role").unwrap(), "reviewer");
        parser.finish().unwrap();
    }
}
