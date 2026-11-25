// SPDX-License-Identifier: MPL-2.0
#pragma once
#include <windows.h>
#include <d2d1.h>
#include <wincodec.h>
#include <wrl/client.h>
#include <vector>
#include <emmintrin.h>

namespace sylva {
inline void convertRow(const unsigned char* src,unsigned width,unsigned char* dst){
    // SSE2 is part of the Windows x64 baseline. Preserve green and alpha;
    // swap red/blue in four pixels at a time, including unaligned subregions.
    unsigned x=0;const auto keep=_mm_set1_epi32(0xff00ff00);const auto low=_mm_set1_epi32(0x000000ff);const auto high=_mm_set1_epi32(0x00ff0000);
    for(;x+4<=width;x+=4){
        auto p=_mm_loadu_si128(reinterpret_cast<const __m128i*>(src+x*4));
        auto q=_mm_or_si128(_mm_and_si128(p,keep),_mm_or_si128(_mm_slli_epi32(_mm_and_si128(p,low),16),_mm_srli_epi32(_mm_and_si128(p,high),16)));
        _mm_storeu_si128(reinterpret_cast<__m128i*>(dst+x*4),q);
    }
    for(;x<width;x++){const auto* p=src+x*4;auto* q=dst+x*4;q[0]=p[2];q[1]=p[1];q[2]=p[0];q[3]=p[3];}
}
inline void bgraToRgba(const unsigned char* src,unsigned stride,unsigned width,unsigned height,unsigned char* dst){
    for(unsigned y=0;y<height;y++)convertRow(src+y*stride,width,dst+y*width*4);
}
struct SoftwarePanel {
    Microsoft::WRL::ComPtr<IWICImagingFactory> factory;
    Microsoft::WRL::ComPtr<IWICBitmap> bitmap;
    Microsoft::WRL::ComPtr<ID2D1RenderTarget> target;
    std::vector<unsigned char> rgba;
    unsigned width=0,height=0;
    HRESULT init(unsigned w,unsigned h,float dpi){
        width=w;height=h;rgba.resize(size_t(w)*h*4);
        HRESULT hr=CoCreateInstance(CLSID_WICImagingFactory,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&factory));if(FAILED(hr))return hr;
        hr=factory->CreateBitmap(w,h,GUID_WICPixelFormat32bppPBGRA,WICBitmapCacheOnLoad,&bitmap);if(FAILED(hr))return hr;
        Microsoft::WRL::ComPtr<ID2D1Factory> d2d;hr=D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED,d2d.GetAddressOf());if(FAILED(hr))return hr;
        auto props=D2D1::RenderTargetProperties(D2D1_RENDER_TARGET_TYPE_SOFTWARE,D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM,D2D1_ALPHA_MODE_PREMULTIPLIED),dpi,dpi);
        return d2d->CreateWicBitmapRenderTarget(bitmap.Get(),props,&target);
    }
    HRESULT lock(Microsoft::WRL::ComPtr<IWICBitmapLock>& locked,UINT& stride,UINT& bytes,BYTE*& data){
        WICRect r{0,0,int(width),int(height)};HRESULT hr=bitmap->Lock(&r,WICBitmapLockRead,&locked);
        if(SUCCEEDED(hr))hr=locked->GetStride(&stride);if(SUCCEEDED(hr))hr=locked->GetDataPointer(&bytes,&data);
        if(SUCCEEDED(hr)&&(stride<width*4||bytes<size_t(stride)*(height-1)+width*4))return E_FAIL;
        return hr;
    }
    HRESULT prepare(unsigned left,unsigned top,unsigned right,unsigned bottom){
        Microsoft::WRL::ComPtr<IWICBitmapLock> locked;UINT stride{},bytes{};BYTE* data{};
        HRESULT hr=lock(locked,stride,bytes,data);if(FAILED(hr))return hr;
        for(unsigned y=top;y<bottom;y++)convertRow(data+y*stride+left*4,right-left,rgba.data()+(y*width+left)*4);
        return S_OK;
    }
    HRESULT mirror(HWND window){
        Microsoft::WRL::ComPtr<IWICBitmapLock> locked;UINT stride{},bytes{};BYTE* data{};
        HRESULT hr=lock(locked,stride,bytes,data);if(FAILED(hr))return hr;
        BITMAPINFO info{};info.bmiHeader.biSize=sizeof(BITMAPINFOHEADER);info.bmiHeader.biWidth=stride/4;info.bmiHeader.biHeight=-int(height);info.bmiHeader.biPlanes=1;info.bmiHeader.biBitCount=32;info.bmiHeader.biCompression=BI_RGB;
        RECT rect{};GetClientRect(window,&rect);HDC dc=GetDC(window);if(!dc)return E_FAIL;
        SetStretchBltMode(dc,HALFTONE);int copied=StretchDIBits(dc,0,0,rect.right,rect.bottom,0,0,width,height,data,&info,DIB_RGB_COLORS,SRCCOPY);ReleaseDC(window,dc);
        return copied==GDI_ERROR?E_FAIL:S_OK;
    }
    HRESULT exportPng(const wchar_t* path,bool raw=false){
        Microsoft::WRL::ComPtr<IWICStream> stream;Microsoft::WRL::ComPtr<IWICBitmapEncoder> encoder;Microsoft::WRL::ComPtr<IWICBitmapFrameEncode> frame;
        Microsoft::WRL::ComPtr<IWICBitmap> rawBitmap;
        HRESULT hr=S_OK;
        if(raw)hr=factory->CreateBitmapFromMemory(width,height,GUID_WICPixelFormat32bppRGBA,width*4,(UINT)rgba.size(),rgba.data(),&rawBitmap);
        if(SUCCEEDED(hr))hr=factory->CreateStream(&stream);
        if(SUCCEEDED(hr))hr=stream->InitializeFromFilename(path,GENERIC_WRITE);
        if(SUCCEEDED(hr))hr=factory->CreateEncoder(GUID_ContainerFormatPng,nullptr,&encoder);
        if(SUCCEEDED(hr))hr=encoder->Initialize(stream.Get(),WICBitmapEncoderNoCache);
        if(SUCCEEDED(hr))hr=encoder->CreateNewFrame(&frame,nullptr);
        if(SUCCEEDED(hr))hr=frame->Initialize(nullptr);
        if(SUCCEEDED(hr))hr=frame->SetSize(width,height);
        WICPixelFormatGUID format=GUID_WICPixelFormat32bppBGRA;
        if(SUCCEEDED(hr))hr=frame->SetPixelFormat(&format);
        if(SUCCEEDED(hr))hr=frame->WriteSource(raw?rawBitmap.Get():bitmap.Get(),nullptr);
        if(SUCCEEDED(hr))hr=frame->Commit();if(SUCCEEDED(hr))hr=encoder->Commit();return hr;
    }
    void close(){target.Reset();bitmap.Reset();factory.Reset();rgba.clear();}
};
}
