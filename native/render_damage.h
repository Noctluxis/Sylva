// SPDX-License-Identifier: MPL-2.0
#pragma once
#include <d2d1.h>
#include <string>
#include <vector>
#include <algorithm>
#include <cmath>
#include "panel_geometry.h"

namespace sylva {
struct PaintCommand {
    bool text=false;
    float x=0,y=0,w=0,h=0,size=0;
    unsigned fill=0,border=0;
    int align=0;
    std::wstring label;
    int icon=-1;
    int preeditStart=-1;
    unsigned preeditLength=0;
    bool operator==(const PaintCommand& b) const {
        return text==b.text&&x==b.x&&y==b.y&&w==b.w&&h==b.h&&size==b.size&&fill==b.fill&&border==b.border&&align==b.align&&label==b.label&&icon==b.icon&&preeditStart==b.preeditStart&&preeditLength==b.preeditLength;
    }
};
struct RenderDamage {
    std::vector<PaintCommand> previous,current;
    unsigned background=0,previousBackground=0;
    bool valid=false,incremental=true;
    void begin(unsigned bg){background=bg;current.clear();}
    D2D1_RECT_F bounds() const {
        if(!incremental||!valid||background!=previousBackground||current.size()!=previous.size())return D2D1::RectF(0,0,logicalWidth,logicalHeight);
        float l=logicalWidth,t=logicalHeight,r=0,b=0;
        auto include=[&](const PaintCommand& c){l=std::min(l,c.x-2);t=std::min(t,c.y-2);r=std::max(r,c.x+c.w+2);b=std::max(b,c.y+c.h+2);};
        for(size_t i=0;i<current.size();i++)if(!(current[i]==previous[i])){include(current[i]);include(previous[i]);}
        if(r<=l||b<=t)return D2D1::RectF(0,0,0,0);
        // Clip on physical pixel boundaries at the panel's 144 DPI. Padding
        // includes rounded borders and antialiasing, preventing stale edges.
        return D2D1::RectF(std::max(0.f,std::floor(l*1.5f)/1.5f),std::max(0.f,std::floor(t*1.5f)/1.5f),std::min(logicalWidth,std::ceil(r*1.5f)/1.5f),std::min(logicalHeight,std::ceil(b*1.5f)/1.5f));
    }
    void commit(){previous.swap(current);previousBackground=background;valid=true;}
};
}
