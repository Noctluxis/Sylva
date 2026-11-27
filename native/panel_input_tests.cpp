// SPDX-License-Identifier: MPL-2.0
#include "panel_input.h"
#include "panel_geometry.h"

// Linked by the Rust test binary. Return the failing line to the test runner.
#define CHECK(condition) do { if (!(condition)) return __LINE__; } while (false)

extern "C" int check_panel_trigger_capture() {
    using sylva::TriggerEdge;
    sylva::PanelButtons left, right;

    // A press from outside may hover, but it must never type or latch on entry.
    CHECK(left.trigger(true, false) == TriggerEdge::none);
    CHECK(left.trigger(true, true) == TriggerEdge::none);
    CHECK(!left.ownsInput(false));
    CHECK(left.trigger(false, true) == TriggerEdge::none);

    CHECK(left.ownsInput(true));
    CHECK(left.trigger(true, true) == TriggerEdge::press);
    CHECK(left.ownsInput(false));
    CHECK(left.trigger(true, false) == TriggerEdge::none);
    CHECK(right.trigger(true, true) == TriggerEdge::press);
    CHECK(left.trigger(false, false) == TriggerEdge::release);
    CHECK(!left.ownsInput(false));
    CHECK(right.ownsInput(false));
    CHECK(right.trigger(true, true) == TriggerEdge::none);
    CHECK(right.trigger(false, false) == TriggerEdge::release);
    CHECK(!right.ownsInput(false));
    CHECK(right.trigger(false, false) == TriggerEdge::none);

    CHECK(left.trigger(true, true) == TriggerEdge::press);
    left = {}; // Panel closed, tracking lost, or action set reset.
    CHECK(!left.ownsInput(false));
    CHECK(left.trigger(false, false) == TriggerEdge::none);
    return 0;
}

extern "C" int check_panel_grip_dwell() {
    using namespace std::chrono_literals;
    sylva::PanelButtons hand;
    const sylva::PanelButtons::Clock::time_point start{};
    CHECK(!hand.startGrab(true, true, start));
    CHECK(hand.trigger(true, true) == sylva::TriggerEdge::press);
    CHECK(!hand.startGrab(true, true, start + 149ms));
    CHECK(!hand.startGrab(false, true, start + 150ms));
    CHECK(!hand.startGrab(true, true, start + 160ms));
    CHECK(!hand.startGrab(true, false, start + 300ms));
    CHECK(!hand.startGrab(true, true, start + 400ms)); // Re-entry while held.
    CHECK(!hand.startGrab(false, true, start + 420ms));
    CHECK(!hand.startGrab(true, false, start + 430ms)); // Held outside first.
    CHECK(!hand.startGrab(true, true, start + 600ms));
    CHECK(!hand.startGrab(false, true, start + 620ms));
    CHECK(!hand.startGrab(true, true, start + 700ms));
    CHECK(hand.startGrab(true, true, start + 850ms));
    CHECK(!hand.startGrab(true, true, start + 950ms)); // Only starts once.
    return 0;
}

extern "C" bool check_settle_panel(vr::HmdMatrix34_t* pose) {
    return sylva::settlePanel(*pose);
}
