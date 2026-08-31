import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons

Item {
  id: root

  property string omarchyPath: ""
  property var shell: null
  property var manifest: null
  readonly property string pluginId: "io.github.tcballard.premonition"
  readonly property string executable: Quickshell.env("PREMONITION_BIN")
    || (Quickshell.env("HOME") + "/.local/bin/premonition")

  property string state: "runtime_missing"
  property string proposalId: ""
  property string repositoryId: ""
  property string correlationId: ""
  property string failureCode: ""
  property int patchBytes: 0
  property int fileCount: 0
  property var recent: []
  property var repositories: []
  property string selectedRepositoryId: ""
  property string rationale: ""
  property string patch: ""
  property string notice: ""
  property bool reviewOpen: false
  property string pendingExplicitAction: ""

  function safeId(value) {
    return /^[A-Za-z0-9._-]{1,64}$/.test(String(value || ""))
  }

  function boundedPayload(payloadJson) {
    var raw = String(payloadJson || "{}")
    if (raw.length > 4096) return {}
    try {
      var value = JSON.parse(raw)
      return value && typeof value === "object" ? value : {}
    } catch (error) {
      return {}
    }
  }

  function open(payloadJson) {
    window.visible = true
    notice = ""
    var payload = boundedPayload(payloadJson)
    if (safeId(payload.repositoryId)) selectedRepositoryId = String(payload.repositoryId)
    var action = String(payload.action || "")
    if (action === "clipboard" || action === "selection") pendingExplicitAction = action
    refreshRepositories()
    refreshStatus()
    Qt.callLater(function() { selectionBox.forceActiveFocus() })
  }

  function close() {
    reviewOpen = false
    window.visible = false
  }

  function requestClose() {
    reviewOpen = false
    if (shell && typeof shell.hide === "function") shell.hide(pluginId)
    else window.visible = false
  }

  function refreshStatus() {
    if (!statusProcess.running) statusProcess.running = true
  }

  function refreshRepositories() {
    if (!repositoriesProcess.running) repositoriesProcess.running = true
  }

  function parseEnvelope(output) {
    var raw = String(output || "")
    if (raw.length === 0 || raw.length > 393216) return null
    try {
      var envelope = JSON.parse(raw)
      if (envelope.contract_version !== 1 || typeof envelope.ok !== "boolean")
        return null
      return envelope
    } catch (error) {
      return null
    }
  }

  function acceptStatus(output) {
    var envelope = parseEnvelope(output)
    if (!envelope || !envelope.ok || !envelope.result
        || envelope.result.kind !== "status") {
      state = "runtime_missing"
      return
    }
    var result = envelope.result
    var allowed = ["idle", "working", "ready", "invalid", "error",
      "runtime_missing", "applying", "recovery_required"]
    var next = String(result.state || "")
    state = allowed.indexOf(next) >= 0 ? next : "error"
    proposalId = safeId(result.proposal_id) ? String(result.proposal_id) : ""
    repositoryId = safeId(result.repository_id) ? String(result.repository_id) : ""
    correlationId = safeId(result.correlation_id) ? String(result.correlation_id) : ""
    failureCode = String(result.failure_code || "")
    patchBytes = Math.max(0, Math.min(262144, Number(result.patch_bytes) || 0))
    fileCount = Math.max(0, Math.min(8, Number(result.file_count) || 0))
    recent = Array.isArray(result.recent) ? result.recent.slice(0, 20) : []
    if (state !== "ready" && state !== "applying") {
      rationale = ""
      patch = ""
      reviewOpen = false
    }
  }

  function acceptRepositories(output) {
    var envelope = parseEnvelope(output)
    if (!envelope || !envelope.ok || !envelope.result
        || envelope.result.kind !== "repositories") return
    var incoming = Array.isArray(envelope.result.repositories)
      ? envelope.result.repositories.slice(0, 64) : []
    var safe = []
    for (var i = 0; i < incoming.length; i++) {
      var entry = incoming[i] || {}
      if (safeId(entry.id)) safe.push({
        id: String(entry.id),
        label: String(entry.label || entry.id).slice(0, 80)
      })
    }
    repositories = safe
    var found = false
    for (var j = 0; j < safe.length; j++)
      if (safe[j].id === selectedRepositoryId) found = true
    if (!found) selectedRepositoryId = safe.length > 0 ? safe[0].id : ""
    if (pendingExplicitAction !== "" && selectedRepositoryId !== "") {
      var action = pendingExplicitAction
      pendingExplicitAction = ""
      submitExplicit(action)
    }
  }

  function submitExplicit(source) {
    if (!safeId(selectedRepositoryId) || actionProcess.running) return
    var flag = source === "selection" ? "--selection" : "--clipboard"
    notice = source === "selection" ? "Sending selected text…" : "Sending clipboard text…"
    actionProcess.command = [
      executable, "submit", "--repo", selectedRepositoryId, flag, "--json"
    ]
    actionProcess.running = true
  }

  function proposalAction(action) {
    if (!safeId(proposalId) || actionProcess.running) return
    notice = action === "apply" ? "Revalidating before Apply…" : ""
    actionProcess.command = [
      executable, "proposal", action, proposalId, "--json"
    ]
    actionProcess.running = true
  }

  function cancelInvestigation() {
    if (actionProcess.running) return
    actionProcess.command = [executable, "cancel", "--json"]
    actionProcess.running = true
  }

  function openReview() {
    if (!safeId(proposalId) || showProcess.running) return
    showProcess.command = [
      executable, "proposal", "show", proposalId, "--json"
    ]
    showProcess.running = true
  }

  function acceptProposal(output) {
    var envelope = parseEnvelope(output)
    if (!envelope || !envelope.ok || !envelope.result
        || envelope.result.kind !== "proposal"
        || envelope.result.proposal_id !== proposalId) {
      notice = "Proposal is no longer available."
      refreshStatus()
      return
    }
    var nextPatch = String(envelope.result.patch || "")
    var nextRationale = String(envelope.result.rationale || "")
    if (nextPatch.length === 0 || nextPatch.length > 262144
        || nextRationale.length === 0 || nextRationale.length > 8192) {
      notice = "Proposal body failed local bounds."
      return
    }
    patch = nextPatch
    rationale = nextRationale
    reviewOpen = true
    Qt.callLater(function() { reviewCloseButton.forceActiveFocus() })
  }

  function acceptAction(output) {
    var envelope = parseEnvelope(output)
    if (!envelope) {
      notice = "Premonition runtime is unavailable."
    } else if (!envelope.ok) {
      notice = envelope.error ? String(envelope.error.message || "Action failed.") : "Action failed."
    } else if (envelope.result) {
      switch (envelope.result.kind) {
        case "accepted": notice = "Investigation started."; break
        case "applied": notice = "Patch applied after revalidation."; reviewOpen = false; break
        case "dismissed": notice = "Proposal dismissed."; reviewOpen = false; break
        case "copied": notice = "Patch copied."; break
        case "cancelled": notice = "Cancellation requested."; break
        default: notice = "Action completed."
      }
    }
    refreshStatus()
  }

  function stateTitle() {
    switch (state) {
      case "idle": return "Idle"
      case "working": return "Investigating"
      case "ready": return "Patch ready"
      case "invalid": return "Invalid candidate"
      case "applying": return "Applying"
      case "runtime_missing": return "Runtime missing"
      case "recovery_required": return "Recovery required"
      default: return "Error"
    }
  }

  function stateDetail() {
    if (state === "idle") return "Send selected or clipboard text explicitly."
    if (state === "working") return "Read-only investigation is in progress."
    if (state === "ready") return fileCount + " file" + (fileCount === 1 ? "" : "s")
      + " · " + patchBytes + " patch bytes"
    if (state === "invalid") return "The candidate failed deterministic validation."
    if (state === "runtime_missing") return "The daemon or configured Codex runtime is unavailable."
    if (state === "recovery_required") return "Resolve the interrupted Apply journal before continuing."
    if (failureCode !== "") return failureCode.replace(/_/g, " ")
    return "Premonition failed safely."
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
    id: repositoriesProcess
    command: [root.executable, "repositories", "--json"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.acceptRepositories(text)
    }
  }

  Process {
    id: showProcess
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.acceptProposal(text)
    }
  }

  Process {
    id: actionProcess
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.acceptAction(text)
    }
  }

  Timer {
    interval: 1200
    running: window.visible || reviewWindow.visible
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refreshStatus()
  }

  FloatingWindow {
    id: window
    title: "Premonition"
    visible: false
    color: Color.popups.background
    implicitWidth: 560
    implicitHeight: 640
    minimumSize: Qt.size(480, 520)

    onVisibleChanged: {
      if (!visible && root.shell && typeof root.shell.hide === "function")
        root.shell.hide(root.pluginId)
    }

    FocusScope {
      anchors.fill: parent
      focus: true
      Keys.onEscapePressed: root.requestClose()

      ColumnLayout {
        anchors.fill: parent
        anchors.margins: Style.space(20)
        spacing: Style.space(14)

        RowLayout {
          Layout.fillWidth: true
          Text {
            textFormat: Text.PlainText
            text: "Premonition"
            color: Color.foreground
            font.family: Style.font.family
            font.pixelSize: Style.font.title
            font.bold: true
          }
          Item { Layout.fillWidth: true }
          Text {
            textFormat: Text.PlainText
            text: root.stateTitle()
            color: root.state === "ready" ? Color.accent
              : (root.state === "invalid" || root.state === "error"
                || root.state === "recovery_required" ? Color.urgent : Color.foreground)
            font.family: Style.font.family
            font.pixelSize: Style.font.body
          }
        }

        Text {
          Layout.fillWidth: true
          textFormat: Text.PlainText
          text: root.stateDetail()
          color: Qt.darker(Color.foreground, 1.35)
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(8)

          Controls.ComboBox {
            id: selectionBox
            Layout.fillWidth: true
            model: root.repositories
            textRole: "label"
            enabled: !actionProcess.running && root.state !== "working"
            onActivated: function(index) {
              if (index >= 0 && index < root.repositories.length)
                root.selectedRepositoryId = root.repositories[index].id
            }
          }

          Controls.Button {
            id: selectionButton
            text: "Send selection"
            enabled: root.selectedRepositoryId !== "" && !actionProcess.running
              && root.state !== "working" && root.state !== "applying"
            KeyNavigation.tab: clipboardButton
            onClicked: root.submitExplicit("selection")
          }

          Controls.Button {
            id: clipboardButton
            text: "Send clipboard"
            enabled: selectionButton.enabled
            KeyNavigation.tab: reviewButton
            onClicked: root.submitExplicit("clipboard")
          }
        }

        Rectangle {
          Layout.fillWidth: true
          height: 1
          color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.18)
        }

        RowLayout {
          Layout.fillWidth: true
          spacing: Style.space(8)

          Controls.Button {
            id: reviewButton
            text: "Review"
            enabled: root.state === "ready" && !showProcess.running
            KeyNavigation.tab: applyButton
            onClicked: root.openReview()
          }
          Controls.Button {
            id: applyButton
            text: "Apply"
            enabled: root.state === "ready" && root.patch !== "" && !actionProcess.running
            KeyNavigation.tab: copyButton
            onClicked: root.proposalAction("apply")
          }
          Controls.Button {
            id: copyButton
            text: "Copy patch"
            enabled: root.state === "ready" && !actionProcess.running
            KeyNavigation.tab: dismissButton
            onClicked: root.proposalAction("copy")
          }
          Controls.Button {
            id: dismissButton
            text: "Dismiss"
            enabled: root.state === "ready" && !actionProcess.running
            KeyNavigation.tab: cancelButton
            onClicked: root.proposalAction("dismiss")
          }
          Controls.Button {
            id: cancelButton
            text: "Cancel"
            enabled: root.state === "working"
            KeyNavigation.tab: selectionButton
            onClicked: root.cancelInvestigation()
          }
        }

        Text {
          Layout.fillWidth: true
          visible: root.notice !== ""
          textFormat: Text.PlainText
          text: root.notice
          color: Color.foreground
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }

        Text {
          textFormat: Text.PlainText
          text: "Recent"
          color: Color.foreground
          font.family: Style.font.family
          font.pixelSize: Style.font.subtitle
          font.bold: true
        }

        ListView {
          Layout.fillWidth: true
          Layout.fillHeight: true
          clip: true
          model: root.recent
          spacing: Style.space(6)

          delegate: Rectangle {
            required property var modelData
            width: ListView.view.width
            height: 44
            radius: Style.cornerRadius
            color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.06)
            RowLayout {
              anchors.fill: parent
              anchors.margins: Style.space(8)
              Text {
                textFormat: Text.PlainText
                text: String(modelData.repository_id || "")
                color: Color.foreground
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
              }
              Item { Layout.fillWidth: true }
              Text {
                textFormat: Text.PlainText
                text: String(modelData.outcome || "")
                color: Qt.darker(Color.foreground, 1.25)
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
              }
            }
          }

          Text {
            anchors.centerIn: parent
            visible: root.recent.length === 0
            textFormat: Text.PlainText
            text: "No completed proposals in this session."
            color: Qt.darker(Color.foreground, 1.45)
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
          }
        }
      }
    }
  }

  PanelWindow {
    id: reviewWindow
    visible: root.reviewOpen
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    exclusionMode: ExclusionMode.Ignore
    WlrLayershell.namespace: "premonition-review"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: root.reviewOpen
      ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

    Rectangle {
      anchors.fill: parent
      color: Qt.rgba(0, 0, 0, 0.72)
    }

    MouseArea {
      anchors.fill: parent
      onClicked: root.reviewOpen = false
    }

    Rectangle {
      width: Math.min(parent.width - Style.space(48), Style.space(1040))
      height: Math.min(parent.height - Style.space(48), Style.space(760))
      anchors.centerIn: parent
      radius: Style.cornerRadius
      color: Color.popups.background

      MouseArea { anchors.fill: parent; onClicked: function(mouse) { mouse.accepted = true } }

      FocusScope {
        anchors.fill: parent
        anchors.margins: Style.space(18)
        focus: true
        Keys.onEscapePressed: root.reviewOpen = false

        ColumnLayout {
          anchors.fill: parent
          spacing: Style.space(10)

          RowLayout {
            Layout.fillWidth: true
            Text {
              textFormat: Text.PlainText
              text: "Candidate patch"
              color: Color.foreground
              font.family: Style.font.family
              font.pixelSize: Style.font.title
              font.bold: true
            }
            Item { Layout.fillWidth: true }
            Controls.Button {
              id: reviewCloseButton
              text: "Close"
              KeyNavigation.tab: reviewApplyButton
              onClicked: root.reviewOpen = false
            }
          }

          Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: root.rationale
            color: Color.foreground
            font.family: Style.font.family
            font.pixelSize: Style.font.body
            wrapMode: Text.WordWrap
          }

          Controls.ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            Controls.TextArea {
              textFormat: TextEdit.PlainText
              text: root.patch
              readOnly: true
              selectByMouse: true
              wrapMode: TextEdit.NoWrap
              color: Color.foreground
              font.family: "monospace"
              font.pixelSize: Style.font.bodySmall
              background: Rectangle {
                color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.04)
                radius: Style.cornerRadius
              }
            }
          }

          RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            Controls.Button {
              id: reviewCopyButton
              text: "Copy patch"
              enabled: !actionProcess.running
              KeyNavigation.tab: reviewDismissButton
              onClicked: root.proposalAction("copy")
            }
            Controls.Button {
              id: reviewDismissButton
              text: "Dismiss"
              enabled: !actionProcess.running
              KeyNavigation.tab: reviewApplyButton
              onClicked: root.proposalAction("dismiss")
            }
            Controls.Button {
              id: reviewApplyButton
              text: "Apply"
              enabled: root.state === "ready" && !actionProcess.running
              KeyNavigation.tab: reviewCloseButton
              onClicked: root.proposalAction("apply")
            }
          }
        }
      }
    }
  }
}
