// Represents the load balancer configuration.
pub struct Config {
    // The configured backends by the user.
    pub Backends: Option<Vec<Backend>>,

    // The backends which are deemed healthy, from the perspective of the load
    // balancer application.
    healthyBackends: Vec<Backend>,
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
