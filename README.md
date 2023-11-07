# Grug Load Balancer (gruglb)

A simplistic L4/L7 load balancer, written in Rust, for [grug brained developers](https://grugbrain.dev/) (me).

# Why?

This is largely a toy project, but also provides a nice segue into being able to use a simple load balancer without many frills for my own projects and to learn
about writing more complex systems in Rust.

It is _very_ likely that this does things which are not optimal, especially in Rust.

## How does it work?

Given a number of pre-defined targets which contains various backend servers, `gruglb` will route traffic between them in round-robin fashion.

When a backend server is deemed unhealthy, by failing a `GET` request to the specified `health_path` for a HTTP target or failing to establish a connection for a TCP target, it is removed
from the routable backends for the specified target. This means that a target with two backends will have all traffic be directed to the single healthy backend until the other server becomes healthy again.

Health checks are conducted at a fixed interval upon the application starting and continue throughout its active lifecycle.

## Features

- Round-robin load balancing of HTTP/TCP connections.
- Health checks for HTTP/TCP targets.

