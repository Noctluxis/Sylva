// SPDX-License-Identifier: MPL-2.0
#pragma once
#include "openvr.h"
#include <cmath>

namespace sylva {
constexpr float logicalWidth=1024.f, logicalHeight=480.f;
struct PanelHit { float x=-1, y=-1; vr::HmdVector3_t point{}; bool hit=false; };
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
