#[cfg(unix)]
mod unix {
    use duet_workbench::rpc::PiClient;
    use std::{process::Command, time::Duration};
    #[test]
    fn fake_subprocess_covers_response_correlation_and_orderly_eof() {
        // Local Python fixture: no Pi, network, or credentials required.
        let mut command = Command::new("python3");
        command.args([
            "-u",
            "-c",
            r#"
import sys, json
for line in sys.stdin.buffer:
    request = json.loads(line)
    print(json.dumps({"type": "queue_update", "steering": [], "followUp": []}), flush=True)
    print(json.dumps({"type":"response","id":request["id"],"command":request["type"],
        "success":True,"data":{"sessionId":"fake","isStreaming":False}}), flush=True)
"#,
        ]);
        let mut client = PiClient::spawn_command(command).unwrap();
        let state = client.get_state(Duration::from_secs(2)).unwrap();
        assert_eq!(state["sessionId"], "fake");
        client.shutdown().unwrap();
    }

    #[test]
    fn child_exit_is_reported_instead_of_waiting_forever() {
        let mut command = Command::new("python3");
        command.args(["-c", "import sys; sys.stdin.buffer.readline(); sys.exit(3)"]);
        let mut client = PiClient::spawn_command(command).unwrap();
        assert!(client.get_state(Duration::from_secs(2)).is_err());
    }
}
