// SPDX-License-Identifier: MPL-2.0
#pragma once
#include <chrono>
#include <optional>

namespace sylva {
enum class TriggerEdge { none, press, release };

struct PanelButtons {
    using Clock = std::chrono::steady_clock;

    bool ownsInput(bool pointing) const { return pointing || captured; }

    TriggerEdge trigger(bool down, bool pointing) {
        if (down == triggerDown) return TriggerEdge::none;
        triggerDown = down;
        if (down) {
            captured = pointing;
            return captured ? TriggerEdge::press : TriggerEdge::none;
        }
        const bool wasCaptured = captured;
        captured = false;
        return wasCaptured ? TriggerEdge::release : TriggerEdge::none;
    }

    // A grip must start on the panel and stay there for the entire dwell.
    // Leaving and re-entering while held cannot arm another grab.
    bool startGrab(bool down, bool eligible, Clock::time_point now) {
        if (!down || !eligible) gripStart.reset();
        else if (!gripDown) gripStart = now;
        gripDown = down;
        if (!gripStart || now - *gripStart < std::chrono::milliseconds(150)) return false;
        gripStart.reset();
        return true;
    }

private:
    bool triggerDown = false;
    bool captured = false;
    bool gripDown = false;
    std::optional<Clock::time_point> gripStart;
};
}
