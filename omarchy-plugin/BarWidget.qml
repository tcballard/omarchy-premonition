import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Item {
  id: root

  property var bar: null
  property string moduleName: ""
  property var settings: ({})

  readonly property string executable: Quickshell.env("PREMONITION_BIN")
    || (Quickshell.env("HOME") + "/.local/bin/premonition")
  property string state: "runtime_missing"

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  function refreshStatus() {
    if (!statusProcess.running) statusProcess.running = true
  }

  function acceptStatus(output) {
    try {
      var envelope = JSON.parse(String(output || ""))
      if (envelope.contract_version !== 1 || envelope.ok !== true
          || !envelope.result || envelope.result.kind !== "status") {
        var code = envelope.error ? String(envelope.error.code || "") : ""
        state = code === "service_unavailable" || code === "runtime_missing"
          ? "runtime_missing" : "error"
        return
      }
      var next = String(envelope.result.state || "")
      var allowed = ["idle", "working", "ready", "invalid", "error",
        "runtime_missing", "applying", "recovery_required"]
      state = allowed.indexOf(next) >= 0 ? next : "error"
    } catch (error) {
      state = "runtime_missing"
    }
  }

  function stateLabel() {
    switch (state) {
      case "idle": return "Premonition · idle"
      case "working": return "Premonition · investigating"
      case "ready": return "Premonition · patch ready"
      case "invalid": return "Premonition · invalid candidate"
      case "applying": return "Premonition · applying"
      case "runtime_missing": return "Premonition · runtime missing"
      case "recovery_required": return "Premonition · recovery required"
      default: return "Premonition · error"
    }
  }

  Process {
    id: statusProcess
    command: [root.executable, "status", "--json"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.acceptStatus(text)
    }
    onExited: function(exitCode) {
      if (exitCode !== 0 && root.state !== "invalid" && root.state !== "error")
        root.state = "runtime_missing"
    }
  }

  Process {
    id: summonProcess
    command: [
      "omarchy-shell", "shell", "summon",
      "io.github.tcballard.premonition", "{}"
    ]
  }

  Timer {
    interval: 1500
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refreshStatus()
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.state === "working" || root.state === "applying" ? "󱥸" : "󰒓"
    tooltipText: root.stateLabel()
    active: root.state === "ready" || root.state === "invalid"
      || root.state === "error" || root.state === "recovery_required"
    activeColor: root.state === "ready" ? Color.accent : Color.urgent
    dimmed: root.state === "runtime_missing"
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.LeftButton && !summonProcess.running)
        summonProcess.running = true
    }
  }
}
