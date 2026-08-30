pragma Singleton

import QtQuick

QtObject {
    readonly property color window: "#111318"
    readonly property color surface: "#181b22"
    readonly property color surfaceRaised: "#20242d"
    readonly property color surfaceHover: "#292e39"
    readonly property color border: "#323844"
    readonly property color borderStrong: "#474f5f"

    readonly property color text: "#f4f5f7"
    readonly property color textMuted: "#a7afbd"
    readonly property color textDim: "#8490a0"

    readonly property color accent: "#80d4b7"
    readonly property color accentStrong: "#a5e7d0"
    readonly property color accentSurface: "#18372f"
    readonly property color warning: "#f4c77a"
    readonly property color warningSurface: "#3a2d19"
    readonly property color danger: "#f39a9f"
    readonly property color dangerSurface: "#3b2025"
    readonly property color info: "#9dbff5"
    readonly property color infoSurface: "#1e2d46"

    readonly property int radiusSmall: 8
    readonly property int radiusMedium: 12
    readonly property int radiusLarge: 18
    readonly property int spaceSmall: 8
    readonly property int spaceMedium: 12
    readonly property int spaceLarge: 18

    function toneForeground(tone) {
        switch (tone) {
        case "positive": return accentStrong;
        case "warning": return warning;
        case "danger": return danger;
        case "info": return info;
        default: return textMuted;
        }
    }

    function toneBackground(tone) {
        switch (tone) {
        case "positive": return accentSurface;
        case "warning": return warningSurface;
        case "danger": return dangerSurface;
        case "info": return infoSurface;
        default: return surfaceRaised;
        }
    }

    function withAlpha(colorValue, alphaValue) {
        return Qt.rgba(colorValue.r, colorValue.g, colorValue.b, alphaValue);
    }
}
