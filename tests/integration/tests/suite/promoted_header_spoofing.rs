// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Praxis Contributors

//! A client cannot pre-set an operator-chosen promotion header to steer
//! header-based routing (praxis-proxy/ai#418).
//!
//! `anthropic_messages_format` promotes the body's `model` into a configurable
//! header that the core `router` matches on. When the body omits `model`, the
//! filter must still remove a client-supplied copy of that header, otherwise
//! the client's value reaches the router as if the filter had derived it.

use praxis_core::config::Config;
use praxis_test_utils::{free_port, http_send, parse_status, start_backend_with_shutdown, start_proxy};

/// A chain that routes on the custom model header `x-route-model`.
fn routing_yaml(proxy_port: u16, privileged_port: u16, default_port: u16) -> String {
    format!(
        r#"
listeners:
  - name: test
    address: "127.0.0.1:{proxy_port}"
    filter_chains: [routing]

filter_chains:
  - name: routing
    filters:
      - filter: anthropic_messages_format
        on_invalid: continue
        headers:
          model: x-route-model
      - filter: router
        routes:
          - path_prefix: "/"
            headers:
              x-route-model: privileged-model
            cluster: privileged
          - path_prefix: "/"
            cluster: default
      - filter: load_balancer
        clusters:
          - name: privileged
            endpoints:
              - "127.0.0.1:{privileged_port}"
          - name: default
            endpoints:
              - "127.0.0.1:{default_port}"

insecure_options:
  allow_private_endpoints: true
"#
    )
}

/// Build a raw Messages POST carrying `body` plus extra header lines.
fn messages_post(body: &str, extra_headers: &[&str]) -> String {
    let mut headers = String::new();
    for line in extra_headers {
        headers.push_str(line);
        headers.push_str("\r\n");
    }
    format!(
        "POST /v1/messages HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Type: application/json\r\n\
         {headers}\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n\
         {body}",
        body.len()
    )
}

/// A body without `model` plus a spoofed `x-route-model` must not reach the
/// privileged backend; the router sees no model header at all.
#[test]
fn spoofed_custom_model_header_does_not_steer_routing_when_body_omits_model() {
    let privileged = start_backend_with_shutdown("privileged-backend");
    let default = start_backend_with_shutdown("default-backend");
    let proxy_port = free_port();
    let config = Config::from_yaml(&routing_yaml(proxy_port, privileged.port(), default.port())).unwrap();
    let proxy = start_proxy(&config);

    let body = r#"{"max_tokens":16,"messages":[{"role":"user","content":"hi"}]}"#;
    let raw = http_send(proxy.addr(), &messages_post(body, &["x-route-model: privileged-model"]));

    assert_eq!(parse_status(&raw), 200, "the request still routes: {raw}");
    assert!(
        raw.contains("default-backend"),
        "a body without a model must take the default route: {raw}"
    );
    assert!(
        !raw.contains("privileged-backend"),
        "a client-supplied x-route-model must never steer routing: {raw}"
    );
}

/// Control: the body-derived model still drives the route.
#[test]
fn body_derived_custom_model_header_routes_to_the_matching_cluster() {
    let privileged = start_backend_with_shutdown("privileged-backend");
    let default = start_backend_with_shutdown("default-backend");
    let proxy_port = free_port();
    let config = Config::from_yaml(&routing_yaml(proxy_port, privileged.port(), default.port())).unwrap();
    let proxy = start_proxy(&config);

    let body = r#"{"model":"privileged-model","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}"#;
    let raw = http_send(proxy.addr(), &messages_post(body, &["x-route-model: ignored"]));

    assert_eq!(parse_status(&raw), 200, "the request routes: {raw}");
    assert!(
        raw.contains("privileged-backend"),
        "the body model must select the privileged cluster: {raw}"
    );
}
