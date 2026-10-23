import QtQuick
import QtQuick.Controls
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "KeyModel.js" as KeyModel

// The settings popout's content: a hero with Badi's mode and a pause
// switch, a live activity line, the week's words, a settings search and one
// keyboard cursor over the controller's typed rows, compact until All
// settings shows every row (with usage details). Keys arrive from the host's
// PanelKeyCatcher through move(), activate(), remove() and focusSearch().
Item {
  id: view

  required property var controller
  required property QtObject bar
  property int cursor: -1
  // The chip under the cursor on a multi-select row.
  property int chip: 0
  // A text input owns the keyboard; the host's key catcher steps aside.
  readonly property bool editing: search.activeFocus || siteInputFocused
  property bool siteInputFocused: false
  property Item siteInput: null
  signal editingFinished()
  signal closeRequested()

  readonly property var client: controller.client
  readonly property var rows: controller.rows()
  readonly property color foreground: bar ? bar.foreground : Commons.Color.foreground
  readonly property color dim: Qt.darker(foreground, 1.45)
  readonly property string fontFamily: bar && bar.fontFamily ? bar.fontFamily : Style.font.family
  readonly property int shown: controller.count("suggestions")
  readonly property int accepted: controller.count("accepted")

  implicitHeight: header.implicitHeight + Style.space(12) + list.implicitHeight + Style.space(12) + footer.implicitHeight

  // The mode, as a glyph and a word, like a status chip.
  readonly property var mode: controller.loading ? {glyph: "◌", text: "Loading", tone: "dim"}
    : !client.brokerReachable ? {glyph: "○", text: "Model offline", tone: "dim"}
    : client.controlPlaneDegraded ? {glyph: "!", text: "Settings need attention", tone: "urgent"}
    : controller.paused && controller.sharingHeld && client.settings.paused !== true
      ? {glyph: "⏸", text: "Held · screen shared", tone: "dim"}
    : controller.paused ? {glyph: "⏸", text: "Paused", tone: "dim"}
    : !controller.anyAppEnabled ? {glyph: "◇", text: "No app allowed", tone: "dim"}
    : {glyph: "⏵⏵", text: "Suggesting", tone: "accent"}

  function reset() {
    controller.confirmation = null
    cursor = -1
    chip = 0
    search.text = ""
    flick.contentY = 0
    controller.showAll = false
    controller.pickedLanguages = null
  }

  function move(dx, dy) {
    if (rows.length === 0) return
    if (dy !== 0) {
      cursor = KeyModel.step(cursor, rows.length, dy)
      chip = 0
      reveal()
      return
    }
    var next = KeyModel.chip(rows[cursor], chip, dx)
    if (next !== null) { chip = next; return }
    var value = KeyModel.choice(rows[cursor], dx)
    if (value !== null) rows[cursor].choose(value)
  }

  function activate() {
    var row = rows[cursor]
    var action = KeyModel.action(row)
    if (action === "focus") { if (siteInput) siteInput.forceActiveFocus() }
    else if (action === "toggle") row.toggle(row.options[Math.min(chip, row.options.length - 1)].value)
    else if (action === "activate") row.activate()
  }

  function remove() {
    var row = rows[cursor]
    if (KeyModel.removable(row)) row.remove()
  }

  function focusSearch() {
    search.forceActiveFocus()
  }

  function reveal() {
    var item = repeater.itemAt(cursor)
    if (!item) return
    var top = item.mapToItem(list, 0, 0).y
    var margin = Style.space(8)
    if (top < flick.contentY + margin) flick.contentY = Math.max(0, top - margin)
    else if (top + item.height > flick.contentY + flick.height - margin)
      flick.contentY = Math.min(Math.max(0, list.implicitHeight - flick.height), top + item.height + margin - flick.height)
  }

  onRowsChanged: if (cursor >= rows.length) cursor = rows.length - 1
  // Expanding or collapsing starts again at the top.
  Connections {
    target: view.controller
    function onShowAllChanged() { view.cursor = -1; view.chip = 0; flick.contentY = 0 }
  }

  // ---- Header: hero, activity, usage, history, search -----------------------

  Column {
    id: header
    anchors { left: parent.left; right: parent.right; top: parent.top }
    spacing: Style.space(12)

    PanelHero {
      id: hero
      width: parent.width
      title: "Badi"
      detail: controller.modelName
      meta: view.mode.glyph + "  " + view.mode.text.toUpperCase()
      foreground: view.foreground
      fontFamily: view.fontFamily
      iconOpacity: client.brokerReachable && !controller.paused ? 1 : 0.5
      iconComponent: Component {
        BadiMark {
          implicitWidth: Style.space(30)
          implicitHeight: Style.space(30)
          ink: view.mode.tone === "accent" ? Commons.Color.accent : view.foreground
        }
      }
      trailingControl: Component {
        ToggleSwitch {
          id: pauseSwitch
          // A screen-share hold is not the user's pause: the switch stays on.
          checked: client.brokerReachable
            && (!client.brokerPaused || (controller.sharingHeld && client.settings.paused !== true))
          busy: client.mutating
          interactive: client.canMutateSettings
          foreground: view.foreground
          onToggled: controller.togglePause()
          PanelToolTip {
            visible: pauseSwitch.containsMouse
            text: pauseSwitch.checked ? "Pause Badi (p)" : "Resume Badi (p)"
            fontFamily: view.fontFamily
          }
        }
      }
    }

    // Live activity: a spinner and a verb while work runs, else a status dot.
    Row {
      id: activityLine
      width: parent.width
      spacing: Style.space(8)
      readonly property bool working: controller.busyVerb !== ""
      readonly property var frames: ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"]
      property int frame: 0
      property double started: 0
      property int elapsed: 0

      onWorkingChanged: { frame = 0; elapsed = 0; started = Date.now() }

      Timer {
        interval: 110
        repeat: true
        running: activityLine.working && view.visible
        onTriggered: {
          activityLine.frame = (activityLine.frame + 1) % activityLine.frames.length
          activityLine.elapsed = Math.floor((Date.now() - activityLine.started) / 1000)
        }
      }

      Text {
        width: Style.space(14)
        text: activityLine.working ? activityLine.frames[activityLine.frame]
          : client.brokerReachable ? "●" : "○"
        color: activityLine.working || view.mode.tone === "accent" ? Commons.Color.accent
          : view.mode.tone === "urgent" ? Commons.Color.urgent : view.dim
        font.family: view.fontFamily
        font.pixelSize: Style.font.body
        horizontalAlignment: Text.AlignHCenter
      }
      Text {
        width: parent.width - Style.space(22)
        elide: Text.ElideRight
        textFormat: Text.PlainText
        text: activityLine.working
          ? controller.busyVerb + "…" + (activityLine.elapsed > 0 ? "  " + activityLine.elapsed + "s" : "")
          : controller.serviceError !== "" ? controller.serviceError
          : !client.brokerReachable ? "Start the model in System below, or run badi service start"
          : view.shown >= 0 ? view.shown + " shown · " + Math.max(0, view.accepted) + " accepted since the model started"
          : controller.statusLabel
        color: activityLine.working ? view.foreground : controller.serviceError !== "" ? Commons.Color.urgent : view.dim
        font.family: view.fontFamily
        font.pixelSize: Style.font.bodySmall
      }
    }

    // Usage, with All settings: a ten-cell acceptance meter, words saved and
    // the last miss.
    Column {
      width: parent.width
      spacing: Style.space(6)
      visible: client.brokerReachable && view.shown > 0 && controller.showAll

      Row {
        spacing: Style.space(10)
        Row {
          spacing: Style.space(2)
          anchors.verticalCenter: parent.verticalCenter
          Repeater {
            model: 10
            Rectangle {
              required property int index
              width: Style.space(9)
              height: Style.space(9)
              radius: Style.cornerRadius > 0 ? Style.space(2) : 0
              readonly property real ratio: view.shown > 0 ? Math.min(1, Math.max(0, view.accepted) / view.shown) : 0
              color: index < Math.round(ratio * 10) ? Commons.Color.accent
                : Qt.rgba(view.foreground.r, view.foreground.g, view.foreground.b, 0.14)
            }
          }
        }
        Text {
          anchors.verticalCenter: parent.verticalCenter
          textFormat: Text.PlainText
          readonly property int words: controller.wordsSaved()
          text: Math.round(Math.min(1, Math.max(0, view.accepted) / view.shown) * 100) + "% accepted"
            + (words >= 0 ? " · ≈ " + words + " words saved" : "")
          color: view.foreground
          font.family: view.fontFamily
          font.pixelSize: Style.font.bodySmall
        }
      }
      Text {
        width: parent.width
        visible: typeof controller.misses.last === "string"
        elide: Text.ElideRight
        textFormat: Text.PlainText
        text: "⎿ last miss: " + (controller.misses.last_meaning || controller.misses.last || "")
        color: view.dim
        font.family: view.fontFamily
        font.pixelSize: Style.font.caption
      }
    }

    // History (badi stats): words saved on each of the last seven days,
    // today's and the week's counts and the apps and sites that saved most.
    // Counts only, kept 90 days; they survive a model restart.
    Column {
      id: history
      width: parent.width
      spacing: Style.space(4)
      visible: client.brokerReachable && controller.statsAvailable
      readonly property var week: controller.statsWeek()
      readonly property var today: controller.statsTotals("today")
      readonly property var lastWeek: controller.statsTotals("last_7_days")
      readonly property var leaders: controller.statsLeaders(3)
      readonly property int peak: week.reduce(function(most, day) { return Math.max(most, day.words) }, 0)

      Row {
        spacing: Style.space(10)
        Row {
          id: weekBars
          height: Style.space(14)
          spacing: Style.space(2)
          anchors.verticalCenter: parent.verticalCenter
          Accessible.name: "Words saved on each of the last seven days"
          Repeater {
            model: history.week
            Rectangle {
              required property var modelData
              required property int index
              anchors.bottom: parent.bottom
              width: Style.space(9)
              height: Style.space(2) + Math.round(Style.space(12) * (history.peak > 0 ? modelData.words / history.peak : 0))
              radius: Style.cornerRadius > 0 ? Style.space(1) : 0
              color: index === history.week.length - 1 ? Commons.Color.accent
                : Qt.rgba(view.foreground.r, view.foreground.g, view.foreground.b, modelData.words > 0 ? 0.45 : 0.14)
            }
          }
        }
        Text {
          anchors.verticalCenter: parent.verticalCenter
          textFormat: Text.PlainText
          text: "≈ " + history.today.words + " words today · " + history.lastWeek.words + " in 7 days"
          color: view.foreground
          font.family: view.fontFamily
          font.pixelSize: Style.font.bodySmall
        }
      }
      Text {
        width: parent.width
        visible: controller.showAll
        elide: Text.ElideRight
        textFormat: Text.PlainText
        text: history.leaders.length > 0
          ? "⎿ most this week: " + history.leaders.map(function(app) { return app.label + " " + app.words }).join(" · ")
          : "⎿ " + history.lastWeek.shown + " shown · " + history.lastWeek.accepted + " accepted in 7 days"
        color: view.dim
        font.family: view.fontFamily
        font.pixelSize: Style.font.caption
      }
    }

    TextField {
      id: search
      width: parent.width
      placeholderText: "Search settings   /"
      foreground: view.foreground
      font.family: view.fontFamily
      font.pixelSize: Style.font.bodySmall
      onTextChanged: { controller.query = text; view.cursor = text ? 0 : -1; flick.contentY = 0 }
      Keys.onEscapePressed: { if (text) { text = ""; view.editingFinished() } else view.closeRequested() }
      Keys.onDownPressed: { view.cursor = 0; view.editingFinished() }
      Keys.onReturnPressed: { if (view.rows.length) view.cursor = 0; view.editingFinished() }
      Accessible.name: "Search Badi settings"
    }
  }

  // ---- Rows -------------------------------------------------------------------

  Flickable {
    id: flick
    anchors { left: parent.left; right: parent.right; top: header.bottom; topMargin: Style.space(12)
      bottom: footer.top; bottomMargin: Style.space(12) }
    contentWidth: width
    contentHeight: list.implicitHeight
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    flickableDirection: Flickable.VerticalFlick
    interactive: contentHeight > height
    ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

    Column {
      id: list
      width: flick.width - Style.space(6)
      spacing: Style.space(6)

      Text {
        width: parent.width
        visible: view.rows.length === 0
        textFormat: Text.PlainText
        text: "No setting matches “" + search.text + "”"
        color: view.dim
        font.family: view.fontFamily
        font.pixelSize: Style.font.bodySmall
      }

      Repeater {
        id: repeater
        model: view.rows

        Column {
          id: rowItem
          required property var modelData
          required property int index
          readonly property bool hasCursor: view.cursor === index
          readonly property bool opensSection: index === 0 || view.rows[index - 1].section !== modelData.section
          width: list.width
          spacing: Style.space(6)

          Item {
            width: parent.width
            visible: rowItem.opensSection && rowItem.modelData.section !== ""
            implicitHeight: sectionColumn.implicitHeight
            Column {
              id: sectionColumn
              width: parent.width
              spacing: Style.space(8)
              topPadding: rowItem.index === 0 ? 0 : Style.space(6)
              PanelSeparator { width: parent.width; foreground: view.foreground; visible: rowItem.index > 0 }
              PanelSectionHeader { text: rowItem.modelData.section.toUpperCase(); foreground: view.foreground; fontFamily: view.fontFamily }
            }
          }

          Loader {
            id: rowLoader
            width: parent.width
            sourceComponent: rowItem.modelData.kind === "toggle" ? toggleRow
              : rowItem.modelData.kind === "choice" ? choiceRow
              : rowItem.modelData.kind === "multi" ? multiRow
              : rowItem.modelData.kind === "site" ? siteRow
              : rowItem.modelData.kind === "field" ? fieldRow : actionRow

            // The keyboard cursor's marker: an accent edge on its row.
            Rectangle {
              anchors { left: parent.left; top: parent.top; bottom: parent.bottom }
              width: Style.space(2)
              color: Commons.Color.accent
              opacity: rowItem.hasCursor ? 1 : 0
              Behavior on opacity { NumberAnimation { duration: 120 } }
            }
          }

          // Declared in the delegate, so each row resolves rowItem directly.
        Component {
          id: toggleRow
          Toggle {
            label: rowItem.modelData.label
            description: (rowItem.modelData.warn ? "⚠ " : "") + rowItem.modelData.detail
            checked: rowItem.modelData.checked
            hasCursor: rowItem.hasCursor
            enabled: rowItem.modelData.enabled
            opacity: rowItem.modelData.enabled ? 1 : 0.55
            foreground: view.foreground
            fontFamily: view.fontFamily
            titleSize: Style.font.body
            onClicked: if (rowItem.modelData.enabled) rowItem.modelData.activate()
            onHovered: function(on) { if (on) view.cursor = rowItem.index }
          }
        }

        Component {
          id: choiceRow
          CursorSurface {
            hasCursor: rowItem.hasCursor
            foreground: view.foreground
            implicitHeight: choiceColumn.implicitHeight + Style.spacing.rowPaddingX
            opacity: rowItem.modelData.enabled ? 1 : 0.55
            HoverHandler { onHoveredChanged: if (hovered) view.cursor = rowItem.index }
            Column {
              id: choiceColumn
              anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter
                leftMargin: Style.spacing.rowPaddingX; rightMargin: Style.spacing.rowPaddingX }
              spacing: Style.space(6)
              Text {
                width: parent.width
                textFormat: Text.PlainText
                text: rowItem.modelData.label
                color: view.foreground
                font.family: view.fontFamily
                font.pixelSize: Style.font.body
              }
              // One chip per option in a Flow, so a long choice wraps
              // instead of running past the popout's edge.
              Flow {
                width: parent.width
                spacing: Style.space(6)
                Repeater {
                  model: rowItem.modelData.options
                  ButtonGroup {
                    required property var modelData
                    options: [modelData]
                    value: rowItem.modelData.value === modelData.value ? modelData.value : ""
                    enabled: rowItem.modelData.enabled
                    focusable: false
                    foreground: view.foreground
                    fontFamily: view.fontFamily
                    fontSize: Style.font.bodySmall
                    onChanged: function(value) { rowItem.modelData.choose(value) }
                  }
                }
              }
              Text {
                width: parent.width
                elide: Text.ElideRight
                textFormat: Text.PlainText
                text: (rowItem.hasCursor ? "h/l · " : "") + rowItem.modelData.detail
                color: view.dim
                font.family: view.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
          }
        }

        // Several of the options at once: one bordered chip per option, lit
        // when chosen; h/l walks them under the cursor and Enter toggles.
        Component {
          id: multiRow
          CursorSurface {
            hasCursor: rowItem.hasCursor
            foreground: view.foreground
            implicitHeight: multiColumn.implicitHeight + Style.spacing.rowPaddingX
            opacity: rowItem.modelData.enabled ? 1 : 0.55
            HoverHandler { onHoveredChanged: if (hovered) view.cursor = rowItem.index }
            Column {
              id: multiColumn
              anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter
                leftMargin: Style.spacing.rowPaddingX; rightMargin: Style.spacing.rowPaddingX }
              spacing: Style.space(6)
              Text {
                width: parent.width
                textFormat: Text.PlainText
                text: rowItem.modelData.label
                color: view.foreground
                font.family: view.fontFamily
                font.pixelSize: Style.font.body
              }
              Flow {
                width: parent.width
                spacing: Style.space(6)
                Repeater {
                  model: rowItem.modelData.options
                  ButtonGroup {
                    required property var modelData
                    required property int index
                    options: [modelData]
                    value: rowItem.modelData.values.indexOf(modelData.value) >= 0 ? modelData.value : ""
                    cursorIndex: rowItem.hasCursor && view.chip === index ? 0 : -1
                    enabled: rowItem.modelData.enabled
                    focusable: false
                    foreground: view.foreground
                    fontFamily: view.fontFamily
                    fontSize: Style.font.bodySmall
                    onChanged: function(value) { view.chip = index; rowItem.modelData.toggle(value) }
                  }
                }
              }
              Text {
                width: parent.width
                wrapMode: Text.WordWrap
                textFormat: Text.PlainText
                text: (rowItem.hasCursor ? "h/l · space · " : "") + rowItem.modelData.detail
                color: rowItem.modelData.detail.indexOf("⚠") >= 0 ? Commons.Color.urgent : view.dim
                font.family: view.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
          }
        }

        Component {
          id: siteRow
          CursorSurface {
            hasCursor: rowItem.hasCursor
            foreground: view.foreground
            implicitHeight: Math.max(siteText.implicitHeight, removeButton.height) + Style.spacing.rowPaddingX
            HoverHandler { onHoveredChanged: if (hovered) view.cursor = rowItem.index }
            TapHandler { onTapped: if (rowItem.modelData.enabled) rowItem.modelData.activate() }
            Text {
              id: siteText
              anchors { left: parent.left; leftMargin: Style.spacing.rowPaddingX; right: removeButton.left
                rightMargin: Style.space(8); verticalCenter: parent.verticalCenter }
              elide: Text.ElideMiddle
              textFormat: Text.PlainText
              text: (rowItem.modelData.checked ? "● " : "○ ") + rowItem.modelData.label + (rowItem.hasCursor ? "   " + rowItem.modelData.detail : "")
              color: rowItem.modelData.checked ? view.foreground : view.dim
              font.family: view.fontFamily
              font.pixelSize: Style.font.bodySmall
            }
            PanelActionButton {
              id: removeButton
              anchors { right: parent.right; rightMargin: Style.space(6); verticalCenter: parent.verticalCenter }
              iconText: "󰅖"
              tooltipText: "Remove this rule (x)"
              foreground: view.foreground
              hoverColor: Commons.Color.urgent
              fontFamily: view.fontFamily
              enabled: rowItem.modelData.enabled
              onClicked: rowItem.modelData.remove()
            }
          }
        }

        Component {
          id: fieldRow
          CursorSurface {
            hasCursor: rowItem.hasCursor
            foreground: view.foreground
            implicitHeight: fieldColumn.implicitHeight + Style.spacing.rowPaddingX
            HoverHandler { onHoveredChanged: if (hovered) view.cursor = rowItem.index }
            Column {
              id: fieldColumn
              anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter
                leftMargin: Style.spacing.rowPaddingX; rightMargin: Style.spacing.rowPaddingX }
              spacing: Style.space(4)
              TextField {
                id: siteField
                width: parent.width
                placeholderText: "https://mail.example.com"
                enabled: rowItem.modelData.enabled
                foreground: view.foreground
                font.family: view.fontFamily
                font.pixelSize: Style.font.bodySmall
                Component.onCompleted: view.siteInput = siteField
                onActiveFocusChanged: view.siteInputFocused = activeFocus
                Keys.onReturnPressed: function(event) {
                  if (controller.setSite(text, (event.modifiers & Qt.ShiftModifier) ? "block" : "allow")) text = ""
                }
                Keys.onEscapePressed: { text = ""; view.editingFinished() }
                Accessible.name: "Website to allow or block"
              }
              Text {
                textFormat: Text.PlainText
                text: rowItem.modelData.detail
                color: view.dim
                font.family: view.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
          }
        }
        Component {
          id: actionRow
          CursorSurface {
            hasCursor: rowItem.hasCursor
            foreground: view.foreground
            opacity: rowItem.modelData.enabled ? 1 : 0.55
            implicitHeight: actionColumn.implicitHeight + Style.spacing.rowPaddingX
            HoverHandler { onHoveredChanged: if (hovered) view.cursor = rowItem.index }
            TapHandler { onTapped: if (rowItem.modelData.enabled) rowItem.modelData.activate() }
            Column {
              id: actionColumn
              anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter
                leftMargin: Style.spacing.rowPaddingX; rightMargin: Style.spacing.rowPaddingX }
              spacing: Style.space(2)
              Text {
                textFormat: Text.PlainText
                text: "› " + rowItem.modelData.label
                color: view.foreground
                font.family: view.fontFamily
                font.pixelSize: Style.font.body
              }
              Text {
                width: parent.width
                elide: Text.ElideRight
                textFormat: Text.PlainText
                text: rowItem.modelData.detail
                color: view.dim
                font.family: view.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
          }
        }

        }
      }
    }
  }

  // ---- Footer: the last result, else the keys --------------------------------

  Column {
    id: footer
    anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
    spacing: Style.space(8)
    readonly property string message: controller.actionMessage || client.message
    readonly property bool failed: controller.actionMessage ? controller.actionFailed : client.messageTone === "danger"

    PanelSeparator { width: parent.width; foreground: view.foreground }
    Text {
      width: parent.width
      wrapMode: Text.WordWrap
      maximumLineCount: 2
      elide: Text.ElideRight
      textFormat: Text.PlainText
      text: footer.message !== "" ? (footer.failed ? "✗ " : "✓ ") + footer.message
        : "j/k h/l move · space toggle · / search · a " + (controller.showAll ? "fewer" : "all") + " · esc close"
      color: footer.message === "" ? view.dim : footer.failed ? Commons.Color.urgent : view.foreground
      font.family: view.fontFamily
      font.pixelSize: Style.font.caption
    }
  }
}
