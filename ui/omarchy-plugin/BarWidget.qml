import QtQuick
import Quickshell
import qs.Ui as Ui
import qs.Commons

Ui.BarWidget {
  id: root
  moduleName: "io.github.ahuray.badi"
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  DesktopPanel { id: panel }

  Ui.BarIconButton {
    id: button
    bar: root.bar
    iconComponent: BadiMark {
      ink: button.active ? button.activeColor : button.foreground
    }
    tooltipText: "Badi · " + panel.statusLabel + "\nLeft-click: settings · Right-click: pause / resume"
    active: panel.ready
    activeColor: Color.accent
    dimmed: panel.paused
    onPressed: (mouseButton) => { if (mouseButton === Qt.RightButton) panel.togglePause(); else panel.toggle() }
    Accessible.name: "Badi writing settings"
    Accessible.role: Accessible.Button
    Accessible.onPressAction: panel.toggle()
  }
}
