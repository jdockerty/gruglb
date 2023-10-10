// Represents the load balancer configuration.
pub struct Config {
    // The configured targets by the user.
    pub Targets: Option<Vec<Target>>,

    // The backends which are deemed healthy, from the perspective of the load
    // balancer application.
    healthyBackends: Vec<Backend>,
}

// A target encapsulates a port that the load balancer listens on for forwarding
// traffic to configured backend servers.
pub struct Target {
    listener_port: String,
    Backends: Option<Vec<Backend>>,
}

// An instance for a backend server that will have traffic routed to.
pub struct Backend {
    host: String,
    port: String,
    healthCheckPath: String,
    // healthCheckInterval: <type>
}

// Choice of a variety of routing algorithms.
pub enum RoutingAlgorithm {
    Simple,
}
