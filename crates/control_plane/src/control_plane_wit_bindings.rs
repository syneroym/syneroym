wit_bindgen::generate!({
    world: "control-plane-service",
    path: "wit/control-plane.wit"
});

struct ControlPlane;

impl exports::syneroym::control_plane::orchestrator::Guest for ControlPlane {
    fn deploy(_service_id: String, _manifest: Vec<u8>) -> Result<(), String> {
        unimplemented!()
    }

    fn stop(_service_id: String) -> Result<(), String> {
        unimplemented!()
    }

    fn remove(_service_id: String) -> Result<(), String> {
        unimplemented!()
    }

    fn readyz(_service_id: String) -> Result<(), String> {
        unimplemented!()
    }
}

impl exports::syneroym::control_plane::health::Guest for ControlPlane {
    fn ping() -> Result<String, String> {
        unimplemented!()
    }
}
