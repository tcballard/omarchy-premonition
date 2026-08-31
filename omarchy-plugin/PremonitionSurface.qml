import QtQuick

Item {
    id: root
    property bool opened: false
    property string omarchyPath: ""
    property var shell
    property var manifest

    function open(payloadJson) {
        void payloadJson
        opened = true
    }

    function close() {
        opened = false
    }
}
