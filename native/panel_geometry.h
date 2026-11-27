// SPDX-License-Identifier: MPL-2.0
#pragma once
#include "openvr.h"
#include <cmath>

namespace sylva {
constexpr float logicalWidth=1024.f, logicalHeight=480.f;
struct PanelHit { float x=-1, y=-1; vr::HmdVector3_t point{}; bool hit=false; };

// Only on an intentional drop. Keep translation and yaw, and preserve each
// deliberate tilt independently. A horizontal panel has no stable yaw.
inline bool settlePanel(vr::HmdMatrix34_t& pose) {
    constexpr float radians = 0.01745329252f;
    const float zx = pose.m[0][2], zy = pose.m[1][2], zz = pose.m[2][2];
    const float horizontal = std::hypot(zx, zz);
    if (horizontal < 0.0001f) return false;
    const float roll = std::atan2(pose.m[1][0], pose.m[1][1]);
    const bool levelRoll = roll != 0 && std::abs(roll) <= 8.f * radians;
    const bool levelPitch = zy != 0 && std::abs(zy) <= std::sin(6.f * radians);
    if (!levelRoll && !levelPitch) return false;

    const float yawX = zx / horizontal, yawZ = zz / horizontal;
    const float rise = levelPitch ? 0.f : zy;
    const float run = levelPitch ? 1.f : horizontal;
    const float right[] = {yawZ, 0.f, -yawX};
    const float up[] = {-rise * yawX, run, -rise * yawZ};
    const float c = levelRoll ? 1.f : std::cos(roll);
    const float s = levelRoll ? 0.f : std::sin(roll);
    for (int i = 0; i < 3; ++i) {
        pose.m[i][0] = right[i] * c + up[i] * s;
        pose.m[i][1] = up[i] * c - right[i] * s;
    }
    if (levelPitch) {
        pose.m[0][2] = yawX;
        pose.m[1][2] = 0.f;
        pose.m[2][2] = yawZ;
    }
    return true;
}
// Place a logical rectangle on the same plane as the hit test. The base aspect
// is width/height; the sliced atlas adjusts it for the cropped source texels.
struct FeedbackQuad {vr::HmdMatrix34_t pose;float width,aspect;};
inline FeedbackQuad feedbackQuad(const vr::HmdMatrix34_t& panel,float width,float x,float y,float w,float h){
    FeedbackQuad out{panel,w/logicalWidth*width,w/h};
    float px=(x+w*.5f-logicalWidth*.5f)*width/logicalWidth;
    float py=(logicalHeight*.5f-y-h*.5f)*width/logicalWidth;
    for(int j=0;j<3;j++)out.pose.m[j][3]+=panel.m[j][0]*px+panel.m[j][1]*py+panel.m[j][2]*.00025f;
    return out;
}

// One plane intersection supplies BOTH the cursor and the logical hit test.
// The flat overlay has square texels, full bounds, and the same aspect as the UI.
// OpenVR intersection vUVs are not assumed to be normalized to texture height.
inline PanelHit intersectPanel(const vr::HmdMatrix34_t& panel, float width,
                              const vr::HmdMatrix34_t& ray) {
    PanelHit out;
    if(!std::isfinite(width) || width<=0) return out;
    float origin[3]{},direction[3]{};
    for(int axis=0;axis<3;axis++) for(int j=0;j<3;j++) {
        origin[axis]+=panel.m[j][axis]*(ray.m[j][3]-panel.m[j][3]);
        direction[axis]+=panel.m[j][axis]*-ray.m[j][2];
    }
    // Only the front face can be typed on. Parallel and behind-ray hits miss.
    if(direction[2]>=-1e-6f || origin[2]<0) return out;
    float distance=-origin[2]/direction[2];
    float localX=origin[0]+distance*direction[0];
    float localY=origin[1]+distance*direction[1];
    out.x=(localX/width+.5f)*logicalWidth;
    out.y=(.5f-localY/(width*logicalHeight/logicalWidth))*logicalHeight;
    if(!std::isfinite(out.x)||!std::isfinite(out.y) ||
       out.x<0||out.y<0||out.x>=logicalWidth||out.y>=logicalHeight) {
        out.x=out.y=-1;return out;
    }
    for(int j=0;j<3;j++)out.point.v[j]=panel.m[j][3]+panel.m[j][0]*localX+panel.m[j][1]*localY;
    out.hit=true;return out;
}
}
