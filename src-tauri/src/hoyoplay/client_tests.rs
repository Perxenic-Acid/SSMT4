use super::*;
use std::{io::Write, net::TcpListener, thread, time::Instant};

fn server(replies: Vec<(&'static str, &'static str)>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut requests = vec![];
        for (status, extra) in replies {
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("本地 HTTP 测试未收到请求：{e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = vec![];
            let mut buffer = [0; 1024];
            while !bytes.ends_with(b"\r\n\r\n") {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0 && bytes.len() < 8192);
                bytes.extend_from_slice(&buffer[..n]);
            }
            requests.push(String::from_utf8(bytes).unwrap());
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 4\r\nConnection: close\r\n{extra}\r\ndata").as_bytes()).unwrap();
        }
        requests
    });
    (format!("http://{address}/manifest"), worker)
}

fn unused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}/manifest", listener.local_addr().unwrap())
}

fn client(proxy: &str) -> Client {
    Client {
        system: Transport::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .proxy(reqwest::Proxy::all(proxy).unwrap())
            .build()
            .unwrap(),
        direct: transport(true).unwrap(),
        routes: Mutex::new(HashMap::new()),
    }
}

#[test]
fn proxy_failure_falls_back_and_remembers_success_while_preserving_resume_headers() {
    let (proxy, proxy_worker) = server(vec![("502 Bad Gateway", "")]);
    let (url, direct_worker) = server(vec![
        ("206 Partial Content", ""),
        ("206 Partial Content", ""),
    ]);
    let http = client(&proxy);
    let url = reqwest::Url::parse(&url).unwrap();
    for offset in [4, 8] {
        assert_eq!(
            http.get_validated(&url, Some(offset))
                .unwrap()
                .text()
                .unwrap(),
            "data"
        );
        assert_eq!(
            http.routes.lock().unwrap().get("127.0.0.1"),
            Some(&Route::Direct)
        );
    }
    assert_eq!(proxy_worker.join().unwrap().len(), 1);
    let requests = direct_worker.join().unwrap();
    assert!(requests[0].to_lowercase().contains("range: bytes=4-"));
    assert!(requests[1].to_lowercase().contains("range: bytes=8-"));
    assert!(requests
        .iter()
        .all(|r| r.to_lowercase().contains("accept-encoding: identity")));
}

#[test]
fn failing_direct_route_can_return_to_system_proxy() {
    let (proxy, worker) = server(vec![("200 OK", "")]);
    let http = client(&proxy);
    http.routes
        .lock()
        .unwrap()
        .insert("127.0.0.1".into(), Route::Direct);
    let url = reqwest::Url::parse(&unused_url()).unwrap();
    assert_eq!(
        http.get_validated(&url, None).unwrap().text().unwrap(),
        "data"
    );
    assert_eq!(
        http.routes.lock().unwrap().get("127.0.0.1"),
        Some(&Route::System)
    );
    worker.join().unwrap();
}

#[test]
fn missing_resource_does_not_retry_or_expose_query_parameters() {
    let (proxy, worker) = server(vec![("404 Not Found", "")]);
    let http = client(&proxy);
    let url =
        reqwest::Url::parse(&format!("{}?password=private-branch-key", unused_url())).unwrap();
    let error = http.get_validated(&url, None).unwrap_err();
    assert!(
        error.contains("404") && error.contains("系统网络路径"),
        "{error}"
    );
    assert!(
        !error.contains("直连路径") && !error.contains("private-branch-key"),
        "{error}"
    );
    worker.join().unwrap();
}

#[test]
fn connection_failures_keep_both_routes_and_the_underlying_socket_reason() {
    let http = client(&unused_url());
    let url = reqwest::Url::parse(&unused_url()).unwrap();
    let error = http.get_validated(&url, None).unwrap_err();
    assert!(
        error.contains("系统网络路径") && error.contains("直连路径"),
        "{error}"
    );
    assert!(
        error.contains(" → ") && error.contains("os error"),
        "{error}"
    );
}

#[test]
fn fallback_keeps_official_https_and_redirect_validation() {
    let http = client(&unused_url());
    for raw in [
        "http://autopatchcn.juequling.com/file",
        "https://juequling.com.evil.test/file",
        "http://127.0.0.1/file",
    ] {
        assert!(http.get(raw, None).unwrap_err().contains("官方 HTTPS"));
    }
    let (url, worker) = server(vec![(
        "302 Found",
        "Location: https://example.invalid/forbidden\r\n",
    )]);
    let http = Client {
        system: transport(true).unwrap(),
        direct: transport(true).unwrap(),
        routes: Mutex::new(HashMap::new()),
    };
    let error = http
        .get_validated(&reqwest::Url::parse(&url).unwrap(), None)
        .unwrap_err();
    assert!(error.contains("重定向不被允许"), "{error}");
    assert!(!error.contains("直连路径"), "{error}");
    worker.join().unwrap();
}
