

# Spawn the fake Go backend TCP servers
run_fake_tcp_backends:
  scripts/spawn_backends.sh

# Fire a few TCP connections via netcat to the LB
hit_proxy:
  #!/bin/sh
  for i in {1..100}; do
      nc --recv-only localhost 9090
  done

hit_full: run_fake_tcp_backends hit_proxy


