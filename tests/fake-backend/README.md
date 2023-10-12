A super simple backend server.

Run the server, ensuring it is listening on a port that is set in the backend configuration for `gruglb`

```bash
go run main.go
```

If you hit `gruglb` on the specified listener, e.g. `curl localhost:9090 --http0.9`, we'll see the server response
come back from the proxy.
