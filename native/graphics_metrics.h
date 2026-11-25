// SPDX-License-Identifier: MPL-2.0
#pragma once
#include <d3d11.h>
#include <wrl/client.h>
#include <array>
#include <vector>
#include <algorithm>
#include <chrono>

namespace sylva {
using Clock=std::chrono::steady_clock;
inline double milliseconds(Clock::time_point start){return std::chrono::duration<double,std::milli>(Clock::now()-start).count();}
struct Samples {
    std::vector<double> values;
    void add(double ms){if(values.size()<4096)values.push_back(ms);}
    void summarize(double* out) const {
        out[0]=(double)values.size();out[1]=out[2]=out[3]=0;
        if(values.empty())return;
        auto sorted=values;std::sort(sorted.begin(),sorted.end());
        out[1]=sorted[sorted.size()/2];out[2]=sorted[sorted.size()*95/100];out[3]=sorted.back();
    }
};
// GPU timestamp queries are read later, with DONOTFLUSH, never polled in a spin
// loop. No texture readback, device-wide wait or extra GPU flush is used.
struct GraphicsMetrics {
    enum Stage { Record, EndDraw, Flush, Submit, GpuDraw, GpuReadyObserved, Mirror, Count };
    struct Query {
        Microsoft::WRL::ComPtr<ID3D11Query> disjoint,start,end;
        bool pending=false;unsigned epoch=0;Clock::time_point submitted;
    };
    std::array<Query,8> queries;
    std::array<Samples,Count> samples;
    unsigned next=0,epoch=0,skipped=0,invalid=0;
    int active=-1;
    bool enabled=true;
    Clock::time_point begin;
    void collect(ID3D11DeviceContext* ctx){
        if(!ctx)return;
        for(auto& q:queries)if(q.pending){
            D3D11_QUERY_DATA_TIMESTAMP_DISJOINT d{};UINT64 a{},b{};
            if(ctx->GetData(q.disjoint.Get(),&d,sizeof(d),D3D11_ASYNC_GETDATA_DONOTFLUSH)!=S_OK)continue;
            if(ctx->GetData(q.start.Get(),&a,sizeof(a),D3D11_ASYNC_GETDATA_DONOTFLUSH)!=S_OK ||
               ctx->GetData(q.end.Get(),&b,sizeof(b),D3D11_ASYNC_GETDATA_DONOTFLUSH)!=S_OK)continue;
            q.pending=false;
            if(q.epoch!=epoch)continue;
            if(!d.Disjoint&&d.Frequency&&b>=a){samples[GpuDraw].add(double(b-a)*1000./double(d.Frequency));samples[GpuReadyObserved].add(milliseconds(q.submitted));}
            else invalid++;
        }
    }
    void start(ID3D11Device* device,ID3D11DeviceContext* ctx){
        begin=Clock::now();active=-1;
        if(!enabled)return;
        collect(ctx);
        auto& q=queries[next];
        if(q.pending){skipped++;return;}
        if(!q.disjoint){
            D3D11_QUERY_DESC desc{D3D11_QUERY_TIMESTAMP_DISJOINT,0};
            if(FAILED(device->CreateQuery(&desc,&q.disjoint)))return;
            desc.Query=D3D11_QUERY_TIMESTAMP;
            if(FAILED(device->CreateQuery(&desc,&q.start))||FAILED(device->CreateQuery(&desc,&q.end))){q.disjoint.Reset();q.start.Reset();q.end.Reset();return;}
        }
        active=int(next);next=(next+1)%queries.size();q.epoch=epoch;
        ctx->Begin(q.disjoint.Get());ctx->End(q.start.Get());
    }
    void end(ID3D11DeviceContext* ctx){
        if(active<0)return;
        auto& q=queries[active];ctx->End(q.end.Get());ctx->End(q.disjoint.Get());q.pending=true;q.submitted=Clock::now();active=-1;
    }
    void add(Stage stage,double ms){if(enabled)samples[stage].add(ms);}
    void reset(){for(auto& s:samples)s.values.clear();epoch++;skipped=invalid=0;enabled=true;}
    void report(double* out,ID3D11DeviceContext* ctx){collect(ctx);for(unsigned i=0;i<Count;i++)samples[i].summarize(out+i*4);out[Count*4]=skipped;out[Count*4+1]=invalid;}
};
}
