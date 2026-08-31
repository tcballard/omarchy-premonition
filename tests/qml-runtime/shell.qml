import QtQuick
import Quickshell

ShellRoot {
  id: test

  readonly property string resultPath: Quickshell.env("PREMONITION_QML_RESULT")
  readonly property string surfaceUrl: Quickshell.env("PREMONITION_SURFACE_URL")
  readonly property string fakeLog: Quickshell.env("PREMONITION_FAKE_LOG")
  readonly property string overlapMarker: Quickshell.env("PREMONITION_FAKE_LOCK") + ".overlap"
  property var failures: []
  property var surface: null
  property int step: 0

  function fail(message) { failures.push(String(message)) }
  function assertTrue(value, message) { if (!value) fail(message) }
  function assertEqual(actual, expected, message) {
    if (actual !== expected) fail(message + " expected=" + expected + " actual=" + actual)
  }
  function quote(value) { return "'" + String(value).replace(/'/g, "'\\''") + "'" }
  function writeResult() {
    var payload = JSON.stringify({ ok: failures.length === 0, failures: failures })
    Quickshell.execDetached(["bash", "-lc", "printf '%s' " + quote(payload) + " > " + quote(resultPath)])
  }

  Item { id: host }
  QtObject {
    id: fakeShell
    property int hides: 0
    function hide(pluginId) { hides++; return pluginId.length > 0 }
  }

  Component.onCompleted: {
    var component = Qt.createComponent(surfaceUrl, Component.PreferSynchronous)
    if (component.status !== Component.Ready) {
      fail("surface failed to load: " + component.errorString())
      writeResult()
      return
    }
    surface = component.createObject(host, { shell: fakeShell, omarchyPath: Quickshell.env("OMARCHY_PATH") })
    if (!surface) {
      fail("surface failed to instantiate: " + component.errorString())
      writeResult()
      return
    }
    surface.open("{}")
    timer.start()
  }

  Timer {
    id: timer
    interval: 220
    repeat: true
    onTriggered: {
      if (!surface) return
      switch (test.step++) {
        case 0:
          test.assertEqual(surface.state, "ready", "status reaches ready")
          test.assertEqual(surface.selectedRepositoryId, "fixture", "allowlist repository selected")
          test.assertEqual(surface.repositories.length, 1, "repository list is bounded")
          surface.refreshStatus()
          surface.refreshStatus()
          break
        case 1:
          var states = ["idle", "working", "ready", "invalid", "error", "runtime_missing", "applying", "recovery_required"]
          for (var i = 0; i < states.length; i++) {
            surface.acceptStatus(JSON.stringify({ contract_version: 1, ok: true, result: { kind: "status", state: states[i], recent: [] } }))
            test.assertEqual(surface.state, states[i], "truthful state " + states[i])
          }
          surface.acceptStatus("not-json")
          test.assertEqual(surface.state, "runtime_missing", "malformed/offline status is honest")
          surface.refreshStatus()
          break
        case 2:
          surface.submitExplicit("selection")
          break
        case 3:
          surface.submitExplicit("clipboard")
          break
        case 4:
          surface.openReview()
          break
        case 5:
          test.assertTrue(surface.reviewOpen, "review overlay opens")
          test.assertTrue(surface.patch.indexOf("<img") >= 0, "hostile patch stays literal")
          test.assertTrue(surface.rationale.indexOf("<b>") >= 0, "hostile rationale stays literal")
          surface.proposalAction("copy")
          break
        case 6:
          test.assertEqual(surface.notice, "Patch copied.", "copy action completes")
          surface.proposalAction("dismiss")
          break
        case 7:
          test.assertEqual(surface.notice, "Proposal dismissed.", "dismiss action completes")
          surface.acceptStatus('{"contract_version":1,"ok":true,"result":{"kind":"status","state":"ready","proposal_id":"p-1","repository_id":"fixture","patch_bytes":91,"file_count":1,"recent":[]}}')
          surface.openReview()
          break
        case 8:
          surface.proposalAction("apply")
          break
        case 9:
          test.assertEqual(surface.notice, "Patch applied after revalidation.", "apply action completes")
          surface.acceptStatus('{"contract_version":1,"ok":true,"result":{"kind":"status","state":"working","repository_id":"fixture","recent":[]}}')
          surface.cancelInvestigation()
          break
        case 10:
          test.assertEqual(surface.notice, "Cancellation requested.", "cancel action completes")
          surface.acceptStatus('{"contract_version":1,"ok":true,"result":{"kind":"status","state":"ready","proposal_id":"p-1","repository_id":"fixture","patch_bytes":91,"file_count":1,"recent":[]}}')
          surface.openReview()
          break
        case 11:
          var process = new XMLHttpRequest()
          process.open("GET", "file://" + fakeLog, false)
          process.send()
          var log = process.responseText
          test.assertTrue(log.indexOf("submit --repo fixture --selection --json") >= 0, "selection is explicit")
          test.assertTrue(log.indexOf("submit --repo fixture --clipboard --json") >= 0, "clipboard is explicit")
          test.assertTrue(log.indexOf("proposal show p-1 --json") >= 0, "review is explicit")
          test.assertTrue(log.indexOf("proposal copy p-1 --json") >= 0, "copy is explicit")
          test.assertTrue(log.indexOf("proposal dismiss p-1 --json") >= 0, "dismiss is explicit")
          test.assertTrue(log.indexOf("proposal apply p-1 --json") >= 0, "apply is explicit")
          test.assertTrue(log.indexOf("cancel --json") >= 0, "cancel is explicit")
          var overlap = new XMLHttpRequest()
          overlap.open("GET", "file://" + overlapMarker, false)
          try { overlap.send() } catch (error) {}
          test.assertTrue(overlap.status === 0 || overlap.status === 404, "polls never overlap")
          timer.stop()
          test.writeResult()
          break
      }
    }
  }
}
