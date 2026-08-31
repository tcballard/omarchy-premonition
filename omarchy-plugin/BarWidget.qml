import QtQuick

Item {
    id: root
    implicitWidth: 28
    implicitHeight: 28

    property var bar
    property string moduleName: ""
    property var settings

    Rectangle {
        anchors.centerIn: parent
        width: 8
        height: 8
        radius: 4
        color: "#9ca3af"
    }
}
