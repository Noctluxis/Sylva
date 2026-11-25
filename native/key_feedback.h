// SPDX-License-Identifier: MPL-2.0
#pragma once
#include "panel_geometry.h"
#include <array>
#include <algorithm>
#include <vector>

namespace sylva {
// A three-slice rounded rectangle: the two 10-unit caps never stretch;
// only the straight 2-unit centre strip changes width. Raster density matches
// the panel (1.5 px / logical unit). The atlas stays resident for both hands.
constexpr unsigned feedbackAtlasWidth=40, feedbackAtlasHeight=432;
constexpr float feedbackScale=1.5f, feedbackCap=10.f;
constexpr int feedbackHeights[]={44,50,56,60,64};
struct FeedbackSlice { FeedbackQuad quad; vr::VRTextureBounds_t uv; };
inline int feedbackRow(float height){
    int row=2;
    for(int h:feedbackHeights){if(std::abs(height-h)<.001f)return row;row+=int(h*feedbackScale)+4;}
    return -1;
}
inline bool feedbackSlices(const vr::HmdMatrix34_t& panel,float width,float x,float y,float w,float h,
                           std::array<FeedbackSlice,3>& out){
    int row=feedbackRow(h);
    if(row<0||w<=2*feedbackCap)return false;
    const float offsets[]={0,feedbackCap,w-feedbackCap},sizes[]={feedbackCap,w-2*feedbackCap,feedbackCap};
    const float texX[]={2,17,20},texW[]={15,3,15};
    for(int i=0;i<3;i++){
        out[i].quad=feedbackQuad(panel,width,x+offsets[i],y,sizes[i],h);
        // OpenVR uses cropped texture aspect * texel aspect, not texel aspect alone.
        out[i].quad.aspect=sizes[i]*feedbackScale/texW[i];
        out[i].uv={texX[i]/feedbackAtlasWidth,float(row)/feedbackAtlasHeight,
                   (texX[i]+texW[i])/feedbackAtlasWidth,(row+h*feedbackScale)/feedbackAtlasHeight};
    }
    return true;
}
inline std::vector<unsigned char> feedbackPixels(){
    std::vector<unsigned char> pixels(feedbackAtlasWidth*feedbackAtlasHeight*4,0);
    for(size_t i=0;i<pixels.size();i+=4)pixels[i]=pixels[i+1]=pixels[i+2]=255;
    for(int height:feedbackHeights){
        int row=feedbackRow(float(height)),pixelHeight=int(height*feedbackScale);
        for(int y=0;y<pixelHeight;y++)for(int x=0;x<33;x++){
            float qx=std::abs((x+.5f)/feedbackScale-11.f)-2.f;
            float qy=std::abs((y+.5f)/feedbackScale-height*.5f)-(height*.5f-9.f);
            float distance=std::hypot(std::max(qx,0.f),std::max(qy,0.f))+std::min(std::max(qx,qy),0.f)-9.f;
            float outer=std::clamp(.5f-distance*feedbackScale,0.f,1.f);
            float inner=std::clamp(.5f-(distance+1.25f)*feedbackScale,0.f,1.f);
            auto* p=&pixels[((row+y)*feedbackAtlasWidth+2+x)*4];
            p[0]=p[1]=p[2]=255;p[3]=(unsigned char)std::lround(255*outer-207*inner);
        }
    }
    return pixels;
}
// Exact nearest pixel of the production atlas, for offline shape regression.
inline unsigned feedbackAlpha(const std::vector<unsigned char>& pixels,float w,float h,float x,float y){
    if(x<0||y<0||x>=w||y>=h||feedbackRow(h)<0||w<=20)return 0;
    float sx=x<10?2+x*feedbackScale:x>=w-10?20+(x-w+10)*feedbackScale:17+(x-10)/(w-20)*3;
    auto tx=std::min(unsigned(sx),feedbackAtlasWidth-1),ty=unsigned(feedbackRow(h)+y*feedbackScale);
    return pixels[(ty*feedbackAtlasWidth+tx)*4+3];
}
}
