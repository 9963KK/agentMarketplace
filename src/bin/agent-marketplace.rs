use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Method;
use reqwest::blocking::Client;
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
    let global_registration_token = parser.take_option("--registration-token");
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
    let registration_token =
        global_registration_token.or_else(|| env::var("AGENT_MARKETPLACE_REGISTRATION_TOKEN").ok());
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

            let response = client.post_json_with_registration_token(
                "/agents/register",
                token.as_deref(),
                None,
                registration_token.as_deref(),
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
        "list-agents" => {
            let alive_only = parser.take_flag("--alive-only");
            let include_deregistered = parser.take_flag("--include-deregistered");
            let limit = parser.take_option("--limit");
            parser.finish()?;

            let mut params = Vec::new();
            if alive_only {
                params.push("alive_only=true".to_string());
            }
            if include_deregistered {
                params.push("include_deregistered=true".to_string());
            }
            if let Some(limit) = limit {
                params.push(format!("limit={}", encode_query(&limit)));
            }
            let path = if params.is_empty() {
                "/agents".to_string()
            } else {
                format!("/agents?{}", params.join("&"))
            };
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
        "relay-create" => {
            let size_bytes = parser
                .take_option("--size-bytes")
                .ok_or_else(|| CliError::usage("relay-create requires --size-bytes"))
                .and_then(|value| parse_u64("--size-bytes", &value))?;
            let ttl_secs = parser
                .take_option("--ttl-secs")
                .map(|value| parse_u64("--ttl-secs", &value))
                .transpose()?;
            let max_downloads = parser
                .take_option("--max-downloads")
                .map(|value| parse_u32("--max-downloads", &value))
                .transpose()?;
            parser.finish()?;

            let response = client.post_json(
                "/relay",
                None,
                None,
                &json!({
                    "size_bytes": size_bytes,
                    "ttl_secs": ttl_secs,
                    "max_downloads": max_downloads,
                }),
            )?;
            print_json(&response)?;
        }
        "relay-upload" => {
            let relay_id = parser
                .take_option("--relay-id")
                .ok_or_else(|| CliError::usage("relay-upload requires --relay-id"))?;
            let relay_token = parser
                .take_option("--relay-token")
                .ok_or_else(|| CliError::usage("relay-upload requires --relay-token"))?;
            let file = parser
                .take_option("--file")
                .ok_or_else(|| CliError::usage("relay-upload requires --file"))?;
            parser.finish()?;

            let response = client.put_bytes_with_relay_token(
                &format!("/relay/{relay_id}"),
                &relay_token,
                fs::read(file)?,
            )?;
            print_json(&response)?;
        }
        "relay-download" => {
            let relay_id = parser
                .take_option("--relay-id")
                .ok_or_else(|| CliError::usage("relay-download requires --relay-id"))?;
            let relay_token = parser
                .take_option("--relay-token")
                .ok_or_else(|| CliError::usage("relay-download requires --relay-token"))?;
            let out = parser
                .take_option("--out")
                .ok_or_else(|| CliError::usage("relay-download requires --out"))?;
            parser.finish()?;

            let bytes =
                client.get_bytes_with_relay_token(&format!("/relay/{relay_id}"), &relay_token)?;
            fs::write(out, bytes)?;
            println!("ok");
        }
        "relay-delete" => {
            let relay_id = parser
                .take_option("--relay-id")
                .ok_or_else(|| CliError::usage("relay-delete requires --relay-id"))?;
            let relay_token = parser
                .take_option("--relay-token")
                .ok_or_else(|| CliError::usage("relay-delete requires --relay-token"))?;
            parser.finish()?;

            let response =
                client.delete_with_relay_token(&format!("/relay/{relay_id}"), &relay_token)?;
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
    base: String,
    client: Client,
}

impl HttpClient {
    fn new(server: String) -> Result<Self, CliError> {
        Ok(Self {
            base: normalize_server(&server)?,
            client: Client::builder().build()?,
        })
    }

    fn get(&self, path: &str, token: Option<&str>) -> Result<Value, CliError> {
        self.request("GET", path, token, None, None, None)
    }

    fn post_empty(
        &self,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, CliError> {
        self.request("POST", path, token, idempotency_key, None, None)
    }

    fn post_json(
        &self,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
        body: &Value,
    ) -> Result<Value, CliError> {
        self.request("POST", path, token, idempotency_key, None, Some(body))
    }

    fn post_json_with_registration_token(
        &self,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
        registration_token: Option<&str>,
        body: &Value,
    ) -> Result<Value, CliError> {
        self.request(
            "POST",
            path,
            token,
            idempotency_key,
            registration_token,
            Some(body),
        )
    }

    fn put_json(
        &self,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
        body: &Value,
    ) -> Result<Value, CliError> {
        self.request("PUT", path, token, idempotency_key, None, Some(body))
    }

    fn put_bytes_with_relay_token(
        &self,
        path: &str,
        relay_token: &str,
        body: Vec<u8>,
    ) -> Result<Value, CliError> {
        let response = self.send_request(
            "PUT",
            path,
            RequestHeaders {
                relay_token: Some(relay_token),
                content_type: Some("application/octet-stream"),
                ..RequestHeaders::default()
            },
            body,
        )?;
        parse_http_value(response.status, &response.body)
    }

    fn get_bytes_with_relay_token(
        &self,
        path: &str,
        relay_token: &str,
    ) -> Result<Vec<u8>, CliError> {
        let response = self.send_request(
            "GET",
            path,
            RequestHeaders {
                relay_token: Some(relay_token),
                ..RequestHeaders::default()
            },
            Vec::new(),
        )?;
        if !(200..300).contains(&response.status) {
            let body = response_body_value(&response.body);
            return Err(CliError::Http {
                status: response.status,
                body,
            });
        }
        Ok(response.body)
    }

    fn delete_with_relay_token(&self, path: &str, relay_token: &str) -> Result<Value, CliError> {
        let response = self.send_request(
            "DELETE",
            path,
            RequestHeaders {
                relay_token: Some(relay_token),
                ..RequestHeaders::default()
            },
            Vec::new(),
        )?;
        parse_http_value(response.status, &response.body)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        idempotency_key: Option<&str>,
        registration_token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value, CliError> {
        let body = body
            .map(serde_json::to_vec)
            .transpose()?
            .unwrap_or_default();
        let response = self.send_request(
            method,
            path,
            RequestHeaders {
                token,
                idempotency_key,
                registration_token,
                content_type: (!body.is_empty()).then_some("application/json"),
                ..RequestHeaders::default()
            },
            body,
        )?;
        parse_http_value(response.status, &response.body)
    }

    fn send_request(
        &self,
        method: &str,
        path: &str,
        headers: RequestHeaders<'_>,
        body: Vec<u8>,
    ) -> Result<HttpResponse, CliError> {
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|_| CliError::usage(format!("invalid HTTP method: {method}")))?;
        let url = format!("{}{}", self.base, path);
        let mut request = self
            .client
            .request(method, url)
            .header("accept", "application/json");
        if let Some(token) = headers.token {
            request = request.bearer_auth(token);
        }
        if let Some(key) = headers.idempotency_key {
            request = request.header("Idempotency-Key", key);
        }
        if let Some(token) = headers.registration_token {
            request = request.header("Registration-Token", token);
        }
        if let Some(token) = headers.relay_token {
            request = request.header("Relay-Token", token);
        }
        if let Some(content_type) = headers.content_type {
            request = request.header("Content-Type", content_type);
        }
        if !body.is_empty() {
            request = request.body(body);
        }

        let response = request.send()?;
        let status = response.status().as_u16();
        let body = response.bytes()?.to_vec();
        Ok(HttpResponse { status, body })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct RequestHeaders<'a> {
    token: Option<&'a str>,
    idempotency_key: Option<&'a str>,
    registration_token: Option<&'a str>,
    relay_token: Option<&'a str>,
    content_type: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn normalize_server(value: &str) -> Result<String, CliError> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        return Err(CliError::usage("server URL is empty"));
    }
    if !(value.starts_with("http://") || value.starts_with("https://")) {
        return Err(CliError::usage(
            "server must start with http:// or https://",
        ));
    }
    Ok(value.to_string())
}

#[cfg(test)]
fn parse_http_raw_response(response: &[u8]) -> Result<HttpResponse, CliError> {
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
    Ok(HttpResponse {
        status,
        body: response[split + 4..].to_vec(),
    })
}

#[cfg(test)]
fn parse_http_response(response: &[u8]) -> Result<Value, CliError> {
    let response = parse_http_raw_response(response)?;
    parse_http_value(response.status, &response.body)
}

fn parse_http_value(status: u16, body: &[u8]) -> Result<Value, CliError> {
    let value = response_body_value(body);
    if !(200..300).contains(&status) {
        return Err(CliError::Http {
            status,
            body: value,
        });
    }
    Ok(value)
}

fn response_body_value(body: &[u8]) -> Value {
    if body.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).trim().to_string()))
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
    Request(reqwest::Error),
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
            CliError::Request(error) => write!(f, "{error}"),
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

impl From<reqwest::Error> for CliError {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(error)
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
  --server <url>          http:// or https://. Default: AGENT_MARKETPLACE_SERVER or http://127.0.0.1:8080
  --token <token>         Default: AGENT_MARKETPLACE_TOKEN or saved credentials
  --agent-id <id>         Default: AGENT_MARKETPLACE_AGENT_ID or saved credentials
  --registration-token <token>
                          Default: AGENT_MARKETPLACE_REGISTRATION_TOKEN

Commands:
  register --agent-id <id> [--name <name>] [--endpoint <url>] [--metadata k=v]
  declare-capabilities --capabilities a,b [--max-concurrency n]
  ping [--busy]
  daemon [--interval seconds] [--busy]
  discover --capability <name> [--include-busy] [--limit n]
  list-agents [--alive-only] [--include-deregistered] [--limit n]
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
  relay-create --size-bytes <n> [--ttl-secs n] [--max-downloads n]
  relay-upload --relay-id <id> --relay-token <token> --file <path>
  relay-download --relay-id <id> --relay-token <token> --out <path>
  relay-delete --relay-id <id> --relay-token <token>
  deregister
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_http_and_https_server_urls() {
        assert_eq!(
            normalize_server("http://127.0.0.1:8080/").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_server("https://platform-server-production-0bc6.up.railway.app").unwrap(),
            "https://platform-server-production-0bc6.up.railway.app"
        );
        assert!(normalize_server("ftp://example.com").is_err());
    }

    #[test]
    fn parses_successful_http_response_as_json() {
        let value =
            parse_http_response(b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\n\r\n{\"ok\":true}")
                .unwrap();

        assert_eq!(value, json!({ "ok": true }));
    }

    #[test]
    fn parses_successful_http_response_as_bytes() {
        let value = parse_http_raw_response(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\n\r\nabc",
        )
        .unwrap();

        assert_eq!(
            value,
            HttpResponse {
                status: 200,
                body: b"abc".to_vec()
            }
        );
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
