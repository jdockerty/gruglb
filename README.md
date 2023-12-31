# Grug Load Balancer (gruglb)

A simplistic L4/L7 load balancer, written in Rust, for [grug brained developers](https://grugbrain.dev/) (me).

# Why?

This is largely a toy project and not intended for production use, but also provides a segue into being able to use a simple load balancer without many frills for my own projects and to learn
about writing more complex systems in Rust.

## Install

Using `cargo` you can install via

```bash
cargo install --git https://github.com/jdockerty/gruglb --bin gruglb
```

Once installed, pass a YAML config file using the `--config` flag, for example

```bash
gruglb --config path/to/config.yml
```

## Features

- Round-robin load balancing of HTTP/HTTPS/TCP connections.
- Health checks for HTTP/HTTPS/TCP targets.
- Graceful termination.
- TLS via termination, backends are still expected to be accessible over HTTP.

## How does it work?

Given a number of pre-defined targets which contains various backend servers, `gruglb` will route traffic between them in round-robin fashion.

When a backend server is deemed unhealthy, by failing a `GET` request to the specified `health_path` for a HTTP target or failing to establish a connection for a TCP target, it is removed
from the routable backends for the specified target. This means that a target with two backends will have all traffic be directed to the single healthy backend until the other server becomes healthy again.

Health checks are conducted at a fixed interval upon the application starting and continue throughout its active lifecycle.

The configuration is defined in YAML, using the `example-config.yaml` that is used for testing, it looks like this:

```yaml
# The interval, in seconds, to conduct HTTP/TCP health checks.
health_check_interval: 2

# Run a graceful shutdown period of 30 seconds to terminate separate worker threads.
# Defaults to true.
graceful_shutdown: true

# Log level information, defaults to 'info'
logging: info

# Defined "targets", the key simply acts as a convenient label for various backend
# servers which are to have traffic routed to them.
targets:

  # TCP target example
  tcpServersA:
    # Either TCP or HTTP, defaults to TCP when not set.
    protocol: 'tcp'

    # Port to bind to for this target.
    listener: 9090

    # Statically defined backend servers.
    backends:
      - host: "127.0.0.1"
        port: 8090
      - host: "127.0.0.1"
        port: 8091

  # HTTP target example
  webServersA:
    protocol: 'http'
    listener: 8080
    backends:
      - host: "127.0.0.1"
        port: 8092
        # A `health_path` is only required for HTTP backends.
        health_path: "/health"
      - host: "127.0.0.1"
        port: 8093
        health_path: "/health"
```

Using the HTTP bound listener of `8080` as our example, if we send traffic to this we expect to see a response back from our
configured backends under `webServersA`. In this instance, the `fake_backend` application is already running.

```bash
# In separate terminal windows (or as background jobs) run the fake backends
fake_backend --id fake-1 --protocol http --port 8092
fake_backend --id fake-2 --protocol http --port 8093

# In your main window, run the load balancer
gruglb --config tests/fixtures/example-config.yaml

# Send some traffic to the load balancer
for i in {1..5}; do curl localhost:8080; echo; done

# You should have the requests routed in a round-robin fashion to the backends.
# The output from the above command should look like this
Hello from fake-2
Hello from fake-1
Hello from fake-2
Hello from fake-1
Hello from fake-2
```

## Performance

_These tests are not very scientific and were simply ran as small experiments to see comparative performance between my implementation
and something that I know is very good._

Using [`bombardier`](https://github.com/codesenberg/bombardier/) as the tool of choice.

### gruglb

<details>

 <summary> Running on localhost </summary>

_CPU: Intel i7-8700 (12) @ 4.600GHz_


Using two [`simplebenchserver`](https://pkg.go.dev/github.com/codesenberg/bombardier@v1.2.6/cmd/utils/simplebenchserver) servers as backends for a HTTP target:

```
bombardier http://127.0.0.1:8080 --latencies --fasthttp -H "Connection: close"
Bombarding http://127.0.0.1:8080 for 10s using 125 connection(s)
[========================================================================================] 10s
Done!
Statistics        Avg      Stdev        Max
  Reqs/sec     42558.30    3130.17   47446.16
  Latency        2.93ms   427.72us    29.29ms
  Latency Distribution
     50%     2.85ms
     75%     3.17ms
     90%     3.61ms
     95%     4.01ms
     99%     5.22ms
  HTTP codes:
    1xx - 0, 2xx - 425267, 3xx - 0, 4xx - 0, 5xx - 0
    others - 0
```
</details>

<details>

 <summary> Running on AWS with m5.xlarge nodes </summary>

This test was performed with 4 nodes: 1 for the load balancer, 2 backend servers running a slightly modified version of `simplebenchserver` which allows binding to `0.0.0.0`, and 1 node used to send traffic internally to the load balancer.

```
bombardier http://172.31.22.113:8080 --latencies --fasthttp -H "Connection: close"
Bombarding http://172.31.22.113:8080 for 10s using 125 connection(s)
[======================================================================================================================================================] 10s
Done!
Statistics        Avg      Stdev        Max
  Reqs/sec     16949.53    9201.05   29354.53
  Latency        7.37ms     6.62ms   103.98ms
  Latency Distribution
     50%     4.99ms
     75%     6.14ms
     90%    14.03ms
     95%    22.23ms
     99%    42.41ms
  HTTP codes:
    1xx - 0, 2xx - 169571, 3xx - 0, 4xx - 0, 5xx - 0
    others - 0
  Throughput:    20.14MB/s
```

</details>


### nginx

<details>

<summary> Configuration </summary>

```
events {
    worker_connections 1024;
}

http {
    server {
        listen 8080;
        location / {
            proxy_pass http://backend;
        }
    }
    upstream backend {
        server 172.31.21.226:8091;
        server 172.31.27.167:8092;
    }
}
```


</details>

<details>

 <summary> Running on localhost </summary>

Using the same two backend servers and a single worker process for `nginx`

```
bombardier http://127.0.0.1:8080 --latencies --fasthttp -H "Connection: close"
Bombarding http://127.0.0.1:8080 for 10s using 125 connection(s)
[========================================================================================] 10s
Done!
Statistics        Avg      Stdev        Max
  Reqs/sec     11996.59     784.99   14555.03
  Latency       10.42ms     2.91ms   226.42ms
  Latency Distribution
     50%    10.37ms
     75%    10.72ms
     90%    11.04ms
     95%    11.22ms
     99%    11.71ms
  HTTP codes:
    1xx - 0, 2xx - 119862, 3xx - 0, 4xx - 0, 5xx - 0
    others - 0
  Throughput:    14.29MB/s
```

Something to note is that `gruglb` does not have the concept of `worker_processes` like `nginx` does.

This was ran with the default of a single process, it performs even better with multiple (~85k req/s).

</details>

<details>

 <summary> Running on AWS with m5.xlarge nodes </summary>

This test was performed with 4 nodes: 1 for the load balancer, 2 backend servers running a slightly modified version of `simplebenchserver` which allows binding to `0.0.0.0`, and 1 node used to send traffic internally to the load balancer.

Again, using the default of `worker_processes 1;`

```
bombardier http://172.31.22.113:8080 --latencies --fasthttp -H "Connection: close"
Bombarding http://172.31.22.113:8080 for 10s using 125 connection(s)
[======================================================================================================================================================] 10s
Done!
Statistics        Avg      Stdev        Max
  Reqs/sec      8207.42    2301.56   11692.24
  Latency       15.22ms     6.57ms   100.47ms
  Latency Distribution
     50%    14.71ms
     75%    15.88ms
     90%    18.67ms
     95%    25.69ms
     99%    49.37ms
  HTTP codes:
    1xx - 0, 2xx - 82117, 3xx - 0, 4xx - 0, 5xx - 0
    others - 0
  Throughput:     9.93MB/s
```

</details>
