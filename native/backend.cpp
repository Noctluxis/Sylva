// SPDX-License-Identifier: MPL-2.0
// Small C ABI bridge to Windows graphics/windowing and Valve's official C++ API.
// All editor, candidate, OSC, layout and application policy lives in Rust.
#define NOMINMAX
#define UNICODE
#define _UNICODE
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <windowsx.h>
#include <imm.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <d2d1_1.h>
#include <dwrite.h>
#include <wincodec.h>
#include <wrl/client.h>
#include <deque>
#include <map>
#include <string>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <algorithm>
#include <chrono>
#include <thread>
#include <mutex>
#include <atomic>
#include <vector>
#include <tuple>
#include <psapi.h>
#include "openvr.h"
#include "panel_geometry.h"
#include "panel_input.h"
#include "key_feedback.h"
#include "graphics_metrics.h"
#include "frame_budget.h"
#include "software_panel.h"
#include "render_damage.h"
#include "raw_swap.h"
#include "svg_icons.h"
#include "lifecycle.h"
using Microsoft::WRL::ComPtr;
constexpr float W=sylva::logicalWidth, H=sylva::logicalHeight;
constexpr UINT TW=UINT(W*1.5f), TH=UINT(H*1.5f);
struct Event { int kind, hand; float x,y; unsigned int value; };
static std::deque<Event> events;
static std::mutex eventMutex;
static HANDLE inputWake=nullptr,vrStopEvent=nullptr;
static std::thread vrThread;
static std::atomic<bool> vrStop{false},textureReady{false},visibleToUi{false},recenterRequested{false};
static std::atomic<float> requestedWidth{.7f};
static std::atomic<bool> requestedBlock{true};
static std::atomic<bool> requestedHaptics{true};
static std::atomic<unsigned> feedbackHands{0};
static std::mutex metricsMutex;
static std::vector<double> pollTimes,frameTimes;
static void queueEvent(Event e) {
    {
        std::lock_guard<std::mutex> lock(eventMutex);
        // Keep the latest motion until a button/cancel boundary, preserving click order.
        if(e.kind==2)for(auto it=events.rbegin();it!=events.rend()&&it->kind==2;++it){
            if(it->hand==e.hand){*it=e;if(inputWake)SetEvent(inputWake);return;}
        }
        events.push_back(e);
    }
    if(inputWake)SetEvent(inputWake);
}
static HWND window;
static std::string actionManifest;
static bool vrInitialized=false;
static bool vrReady=false, visible=true, placed=false, offscreen=false,hiddenProbe=false;
static sylva::PanelButtons panelButtons[2];
static bool armed[2]{};
static int grabHand=-1;
static vr::HmdMatrix34_t panel{}, grabOffset{};
static vr::VROverlayHandle_t overlay=0, cursors[2]{};
static vr::VROverlayHandle_t panelSlots[2]{};
static bool panelShown[2]{};
static sylva::RawSwap rawSwap;
static int handoffSlot=-1;
static std::chrono::steady_clock::time_point handoffUntil{};
static std::atomic<uint64_t> rawSwaps{0};
static uint64_t frontChecks=0,frontMissing=0;
static vr::VROverlayHandle_t keyFeedback[2][3]{};
static bool keyFeedbackVisible[2]{},feedbackPressed[2]{};
static std::atomic<bool> keyFeedbackLoaded[2][3]{};
static std::vector<unsigned char> feedbackPixels;
struct KeyRegion {float x,y,w,h;};
static std::mutex regionsMutex;
static std::vector<KeyRegion> keyRegions;
static KeyRegion feedbackSize[2]{},flashRegion[2]{};
static float feedbackWidth[2][3]{};
static vr::HmdMatrix34_t feedbackTransform[2][3]{};
static std::chrono::steady_clock::time_point flashUntil[2]{};
static vr::VRActionSetHandle_t actions=0;
static vr::VRActionHandle_t clickAction=0, grabAction=0, toggleAction=0, hapticAction=0;
static vr::VRInputValueHandle_t hands[2]{};
static float panelWidth=.7f;
static bool blockInput=true;
static bool dashboardVisible=false, cursorVisible[2]{}, desktopDirty=false;
static HANDLE waitTimer=nullptr,frameTimer=nullptr;
static unsigned frameTimeout=12;
static float predictionSeconds=0.011f;
static void vrLoop();
struct ControllerTip { vr::TrackedDeviceIndex_t index=vr::k_unTrackedDeviceIndexInvalid; vr::HmdMatrix34_t transform{}; bool valid=false; };
static ControllerTip controllerTips[2];
static std::chrono::steady_clock::time_point lastDesktopPresent{};
static char lastError[512]{};
static ComPtr<ID3D11Device> device;
static ComPtr<ID3D11DeviceContext> context;
static ComPtr<ID3D11Texture2D> texture;
static ComPtr<IDXGISwapChain1> swap;
static ComPtr<ID2D1DeviceContext> draw;
static ComPtr<ID2D1RenderTarget> paint;
static ComPtr<ID2D1PathGeometry> iconGeometry[5];
static ComPtr<ID2D1StrokeStyle> iconStroke;
static sylva::SoftwarePanel software;
static bool cpuPanel=false;
static sylva::RenderDamage damage;
static bool replaying=false;
static sylva::Samples prepareTimes;
static std::atomic<bool> rawFailed{false};
static std::atomic<uint64_t> rawSubmittedAt{0},rawLoaded{0};
static bool rawDirty=false;
static uint64_t rawSubmitted=0,rawCoalesced=0;
static int loadedImageSlot=-1;
static sylva::Samples rawLoadTimes;
static float displayHz=90;
static uint64_t nowNanoseconds(){return (uint64_t)std::chrono::duration_cast<std::chrono::nanoseconds>(sylva::Clock::now().time_since_epoch()).count();}
static void panelTransform(){for(auto h:panelSlots)if(h)vr::VROverlay()->SetOverlayTransformAbsolute(h,vr::TrackingUniverseStanding,&panel);}
static void panelWidthSetting(){for(auto h:panelSlots)if(h)vr::VROverlay()->SetOverlayWidthInMeters(h,panelWidth);}
static bool panelTextureAvailable(int slot){uint32_t w{},h{};return slot>=0&&vr::VROverlay()->GetOverlayTextureSize(panelSlots[slot],&w,&h)==vr::VROverlayError_None&&w==TW&&h==TH;}
static bool showPanelSlot(int slot,bool show){
    if(slot<0||!panelSlots[slot])return false;
    show=show&&!hiddenProbe;if(panelShown[slot]==show)return true;
    auto e=show?vr::VROverlay()->ShowOverlay(panelSlots[slot]):vr::VROverlay()->HideOverlay(panelSlots[slot]);
    if(e!=vr::VROverlayError_None){rawFailed.store(true);return false;}
    panelShown[slot]=show;return true;
}
static void presentPanel(bool show){
    if(!cpuPanel){showPanelSlot(0,show&&textureReady.load());return;}
    int ready=rawSwap.ready(),front=rawSwap.front();auto now=std::chrono::steady_clock::now();
    if(ready>=0&&handoffSlot<0){
        // Draw the new image above the old one, retaining the old texture for
        // one display period. Never hide the last complete frame first.
        vr::VROverlay()->SetOverlaySortOrder(panelSlots[ready],1);
        if(showPanelSlot(ready,show)){handoffSlot=ready;handoffUntil=now+std::chrono::microseconds((long long)std::ceil(1000000./displayHz));}
    }
    if(handoffSlot>=0&&now>=handoffUntil){
        if(front<0||showPanelSlot(front,false)){
            vr::VROverlay()->SetOverlaySortOrder(panelSlots[handoffSlot],0);
            rawSwap.presented(handoffSlot);rawSwaps.fetch_add(1);handoffSlot=-1;textureReady.store(true);if(inputWake)SetEvent(inputWake);
            front=rawSwap.front();
        }
    }
    for(int i=0;i<2;i++)showPanelSlot(i,show&&(i==front||i==handoffSlot));
}
static ComPtr<ID2D1Bitmap1> target, desktopTarget;
static ComPtr<ID2D1SolidColorBrush> brush;
static ComPtr<IDWriteFactory> fonts;
static std::map<int,ComPtr<IDWriteTextFormat>> formats;
static sylva::GraphicsMetrics graphicsMetrics;
static std::string adapterDescription,runtimeVersion;
static sylva::Samples waitTimes;
static unsigned waitTimeouts=0,waitErrors=0;
using LayoutKey=std::tuple<std::wstring,int,float,float,int>;
struct CachedLayout {ComPtr<IDWriteTextLayout> value;uint64_t used=0;};
static std::map<LayoutKey,CachedLayout> layouts;
static uint64_t layoutUse=0,layoutHits=0,layoutMisses=0;
static bool textCache=true;
struct WidthEntry {float width;uint64_t used;};
static std::map<std::pair<std::wstring,int>,WidthEntry> textWidths;
static uint64_t widthUse=0;
static bool check(HRESULT hr,const char* name) {
    if(SUCCEEDED(hr)) return true;
    snprintf(lastError,sizeof(lastError),"%s: 0x%08lx",name,(unsigned long)hr); return false;
}
static D2D1_COLOR_F color(unsigned int c) { return D2D1::ColorF(((c>>16)&255)/255.f,((c>>8)&255)/255.f,(c&255)/255.f,((c>>24)&255)/255.f); }
static vr::HmdMatrix34_t mul(const vr::HmdMatrix34_t&a,const vr::HmdMatrix34_t&b) {
    vr::HmdMatrix34_t r{};
    for(int i=0;i<3;i++)for(int j=0;j<4;j++) { for(int k=0;k<3;k++)r.m[i][j]+=a.m[i][k]*b.m[k][j]; if(j==3)r.m[i][j]+=a.m[i][3]; }
    return r;
}
static vr::HmdMatrix34_t inverse(const vr::HmdMatrix34_t&a) {
    vr::HmdMatrix34_t r{};
    for(int i=0;i<3;i++){for(int j=0;j<3;j++)r.m[i][j]=a.m[j][i];for(int j=0;j<3;j++)r.m[i][3]-=r.m[i][j]*a.m[j][3];} return r;
}
static bool feedbackLoaded(int hand){return keyFeedbackLoaded[hand][0]&&keyFeedbackLoaded[hand][1]&&keyFeedbackLoaded[hand][2];}
static void hideKeyFeedback(int hand){if(keyFeedbackVisible[hand])for(auto handle:keyFeedback[hand])if(handle)vr::VROverlay()->HideOverlay(handle);keyFeedbackVisible[hand]=false;}
static void showCursor(int i,bool show) {
    if(!show)hideKeyFeedback(i);
    if(!cursors[i]||cursorVisible[i]==show)return;
    if(show)vr::VROverlay()->ShowOverlay(cursors[i]);else vr::VROverlay()->HideOverlay(cursors[i]);
    cursorVisible[i]=show;
}
static void updateKeyFeedback(int hand,const sylva::PanelHit& hit,bool pressedHere){
    KeyRegion region{};
    if(hit.hit&&grabHand<0&&textureReady.load()&&feedbackLoaded(hand)){
        std::lock_guard<std::mutex> lock(regionsMutex);
        for(const auto& r:keyRegions)if(hit.x>=r.x&&hit.x<r.x+r.w&&hit.y>=r.y&&hit.y<r.y+r.h){region=r;break;}
    }
    std::array<sylva::FeedbackSlice,3> slices{};
    if(region.w<=0||!sylva::feedbackSlices(panel,panelWidth,region.x,region.y,region.w,region.h,slices)){hideKeyFeedback(hand);return;}
    auto now=std::chrono::steady_clock::now();
    if(pressedHere){flashRegion[hand]=region;flashUntil[hand]=now+std::chrono::milliseconds(100);}
    bool pressed=now<flashUntil[hand]&&region.x==flashRegion[hand].x&&region.y==flashRegion[hand].y&&region.w==flashRegion[hand].w&&region.h==flashRegion[hand].h;
    // The feedback image is uploaded once at startup, just like the cursor.
    // Hover/press changes only transform/color/alpha, never pixels or UI draw.
    auto& size=feedbackSize[hand];
    for(int part=0;part<3;part++){
        auto handle=keyFeedback[hand][part];auto& slice=slices[part];auto& quad=slice.quad;
        if(size.h!=region.h)vr::VROverlay()->SetOverlayTextureBounds(handle,&slice.uv);
        if(size.w!=region.w||size.h!=region.h)vr::VROverlay()->SetOverlayTexelAspect(handle,quad.aspect);
        // Width settings can change while hovering the same key.
        if(feedbackWidth[hand][part]!=quad.width){vr::VROverlay()->SetOverlayWidthInMeters(handle,quad.width);feedbackWidth[hand][part]=quad.width;}
        if(!keyFeedbackVisible[hand]||feedbackPressed[hand]!=pressed){
            vr::VROverlay()->SetOverlayColor(handle,pressed?.20f:.46f,pressed?.64f:.70f,pressed?.40f:.55f);
            vr::VROverlay()->SetOverlayAlpha(handle,pressed?.85f:.48f);
        }
        if(!keyFeedbackVisible[hand]||std::memcmp(&quad.pose,&feedbackTransform[hand][part],sizeof(quad.pose))){vr::VROverlay()->SetOverlayTransformAbsolute(handle,vr::TrackingUniverseStanding,&quad.pose);feedbackTransform[hand][part]=quad.pose;}
        if(!keyFeedbackVisible[hand])vr::VROverlay()->ShowOverlay(handle);
    }
    size=region;feedbackPressed[hand]=pressed;keyFeedbackVisible[hand]=true;
}
static void resetHands() {
    grabHand=-1;
    for(int i=0;i<2;i++){armed[i]=false;panelButtons[i]={};flashUntil[i]={};queueEvent({7,i,0,0,0});showCursor(i,false);}
}
static bool recenter() {
    if(!vrReady)return false;
    vr::TrackedDevicePose_t poses[vr::k_unMaxTrackedDeviceCount]{};
    vr::VRSystem()->GetDeviceToAbsoluteTrackingPose(vr::TrackingUniverseStanding,0,poses,vr::k_unMaxTrackedDeviceCount);
    if(!poses[0].bPoseIsValid)return false;
    auto h=poses[0].mDeviceToAbsoluteTracking;
    float fx=-h.m[0][2],fz=-h.m[2][2],n=std::hypot(fx,fz);if(n<.1f)return false;fx/=n;fz/=n;
    panel={};panel.m[0][0]=-fz;panel.m[2][0]=fx;panel.m[1][1]=1;panel.m[0][2]=-fx;panel.m[2][2]=-fz;
    panel.m[0][3]=h.m[0][3]+fx*.8f;panel.m[1][3]=h.m[1][3]-.25f;panel.m[2][3]=h.m[2][3]+fz*.8f;
    panelTransform();placed=true;return true;
}
static LRESULT CALLBACK wndproc(HWND hwnd,UINT msg,WPARAM wp,LPARAM lp) {
    if(msg==lifecycle::taskbarCreated && msg){
        lifecycle::trayAdded=false;
        if(!lifecycle::addTray(hwnd))lifecycle::show(hwnd);
        return 0;
    }
    if(msg==lifecycle::activateMessage){
        if(wp==1)queueEvent({10,2,0,0,0});
        else{desktopDirty=true;lifecycle::show(hwnd);}
        return 0;
    }
    if(msg==lifecycle::trayMessage){
        UINT command=lifecycle::popup(hwnd,lp,wp);
        if(command)SendMessageW(hwnd,WM_COMMAND,command,0);
        return 0;
    }
    RECT r{};GetClientRect(hwnd,&r);
    float x=GET_X_LPARAM(lp)*W/std::max(1L,r.right),y=GET_Y_LPARAM(lp)*H/std::max(1L,r.bottom);
    switch(msg){
    case WM_COMMAND:
        if(LOWORD(wp)==lifecycle::showCommand){desktopDirty=true;lifecycle::show(hwnd);}
        else if(LOWORD(wp)==lifecycle::exitCommand)queueEvent({1,2,0,0,0});
        return 0;
    case WM_CLOSE:
        if(lifecycle::closeToTray&&lifecycle::trayAdded){ShowWindow(hwnd,SW_HIDE);queueEvent({8,2,0,0,0});}
        else queueEvent({1,2,0,0,0});
        return 0;
    case WM_PAINT:{PAINTSTRUCT p;BeginPaint(hwnd,&p);EndPaint(hwnd,&p);desktopDirty=true;queueEvent({6,2,0,0,0});return 0;}
    case WM_ERASEBKGND:return 1;
    case WM_MOUSEMOVE:queueEvent({2,2,x,y,0});return 0;
    case WM_LBUTTONDOWN:SetCapture(hwnd);queueEvent({3,2,x,y,0});return 0;
    case WM_LBUTTONUP:queueEvent({4,2,x,y,0});ReleaseCapture();return 0;
    case WM_CAPTURECHANGED:case WM_KILLFOCUS:queueEvent({7,2,0,0,0});return 0;
    case WM_CHAR: if(wp>=32||wp==8||wp==13||wp==27)queueEvent({5,2,0,0,(unsigned)wp});return 0;
    case WM_KEYDOWN:if(wp==VK_CAPITAL)queueEvent({5,2,0,0,0x110000});return 0;
    }
    return DefWindowProcW(hwnd,msg,wp,lp);
}
static void stopVr(){
    if(vrInitialized){
        if(vr::VROverlay()){
            for(auto& c:cursors){if(c)vr::VROverlay()->DestroyOverlay(c);c=0;}
            for(auto& hand:keyFeedback)for(auto& k:hand){if(k)vr::VROverlay()->DestroyOverlay(k);k=0;}
            for(auto& h:panelSlots){if(h){vr::VROverlay()->ClearOverlayTexture(h);vr::VROverlay()->DestroyOverlay(h);}h=0;}
            if(overlay&&!vrReady)vr::VROverlay()->DestroyOverlay(overlay);
        }
        vr::VR_Shutdown();vrInitialized=false;
    }
    vrReady=false;overlay=0;
}
static bool startVr(const char* manifest,int desktopOnly){
    if((desktopOnly==0||desktopOnly==4||hiddenProbe) && vr::VR_IsHmdPresent()) {
        vr::EVRInitError e=vr::VRInitError_None;vr::VR_Init(&e,vr::VRApplication_Overlay);vrInitialized=e==vr::VRInitError_None;
        if(e==vr::VRInitError_None && vr::VROverlay() && vr::VRInput()) {
            // Autolaunch may hand off from a short-lived second process to
            // the tray instance. Associate the surviving PID before actions.
            if(!hiddenProbe&&vr::VRApplications()&&vr::VRApplications()->IsApplicationInstalled("local.sylva.keyboard"))
                vr::VRApplications()->IdentifyApplication(GetCurrentProcessId(),"local.sylva.keyboard");
            std::string panelKey=hiddenProbe?"local.sylva.hidden-probe."+std::to_string(GetCurrentProcessId()):"local.sylva.keyboard";
            auto err=vr::VROverlay()->CreateOverlay(panelKey.c_str(),"Sylva VR",&overlay);
            bool ok=err==vr::VROverlayError_None;
            if(!hiddenProbe){ok=ok && vr::VRInput()->SetActionManifestPath(manifest)==vr::VRInputError_None;
            ok=ok && vr::VRInput()->GetActionSetHandle("/actions/main",&actions)==vr::VRInputError_None;
            ok=ok && vr::VRInput()->GetActionHandle("/actions/main/in/click",&clickAction)==vr::VRInputError_None;
            ok=ok && vr::VRInput()->GetActionHandle("/actions/main/in/grab",&grabAction)==vr::VRInputError_None;
            ok=ok && vr::VRInput()->GetActionHandle("/actions/main/in/toggle",&toggleAction)==vr::VRInputError_None;
            vr::VRInput()->GetActionHandle("/actions/main/out/haptic",&hapticAction);
            vr::VRInput()->GetInputSourceHandle("/user/hand/left",&hands[0]);vr::VRInput()->GetInputSourceHandle("/user/hand/right",&hands[1]);
            }
            if(ok){vrReady=true;panelSlots[0]=overlay;
                if(desktopOnly!=4){auto backKey=panelKey+".back";if(vr::VROverlay()->CreateOverlay(backKey.c_str(),"Sylva panel back buffer",&panelSlots[1])!=vr::VROverlayError_None){snprintf(lastError,sizeof(lastError),"Create back buffer overlay failed");return 0;}}
                panelWidthSetting();for(auto h:panelSlots)if(h)vr::VROverlay()->SetOverlayInputMethod(h,vr::VROverlayInputMethod_None);
                vr::VRTextureBounds_t bounds{0,0,1,1};vr::VROverlay()->SetOverlayTextureBounds(overlay,&bounds);vr::VROverlay()->SetOverlayTexelAspect(overlay,1.f);
                float hz=vr::VRSystem()->GetFloatTrackedDeviceProperty(0,vr::Prop_DisplayFrequency_Float);
                if(!std::isfinite(hz)||hz<30)hz=90;hz=std::clamp(hz,30.f,240.f);
                displayHz=hz;runtimeVersion=vr::VRSystem()->GetRuntimeVersion();
                frameTimeout=(unsigned)std::ceil(1000.f/hz);predictionSeconds=1.f/hz;
                recenter();
                if(!hiddenProbe)feedbackPixels=sylva::feedbackPixels();
                for(int i=0;i<2&&!hiddenProbe;i++){
                    std::string key="local.sylva.cursor"+std::to_string(i);
                    if(vr::VROverlay()->CreateOverlay(key.c_str(),"Sylva pointer",&cursors[i])==vr::VROverlayError_None){
                        unsigned char pixels[32*32*4]{};
                        for(int y=0;y<32;y++)for(int x=0;x<32;x++){float d=std::hypot(x-15.5f,y-15.5f);auto*p=&pixels[(y*32+x)*4];p[0]=i?244:70;p[1]=i?178:206;p[2]=i?76:218;p[3]=d<10?255:0;}
                        vr::VROverlay()->SetOverlayRaw(cursors[i],pixels,32,32,4);vr::VROverlay()->SetOverlayWidthInMeters(cursors[i],.012f);vr::VROverlay()->SetOverlaySortOrder(cursors[i],10);
                    }
                    for(int part=0;part<3;part++){
                        std::string feedbackKey="local.sylva.key-feedback"+std::to_string(i)+"."+std::to_string(part);
                        auto& handle=keyFeedback[i][part];
                        if(vr::VROverlay()->CreateOverlay(feedbackKey.c_str(),"Sylva key feedback",&handle)!=vr::VROverlayError_None){snprintf(lastError,sizeof(lastError),"Create key feedback overlay failed");return 0;}
                        if(vr::VROverlay()->SetOverlayRaw(handle,feedbackPixels.data(),sylva::feedbackAtlasWidth,sylva::feedbackAtlasHeight,4)!=vr::VROverlayError_None){snprintf(lastError,sizeof(lastError),"Upload key feedback image failed");return 0;}
                        vr::VROverlay()->SetOverlaySortOrder(handle,5);
                        vr::VROverlay()->SetOverlayInputMethod(handle,vr::VROverlayInputMethod_None);
                    }
                }
            }else{snprintf(lastError,sizeof(lastError),"SteamVR overlay/action initialization failed (%d)",err);if(overlay)vr::VROverlay()->DestroyOverlay(overlay);overlay=0;vr::VR_Shutdown();vrInitialized=false;}
        }else snprintf(lastError,sizeof(lastError),"SteamVR: %s",vr::VR_GetVRInitErrorAsEnglishDescription(e));
    }
    return vrReady;
}
extern "C" const char* native_error(){return lastError;}
// A utility-only subprocess uses this API. It does not start drivers or touch
// the active renderer/input thread's global OpenVR context.
extern "C" int native_autostart(const char* manifest,const char* key,int enabled,int temporary){
    vr::EVRInitError init=vr::VRInitError_None;vr::VR_Init(&init,vr::VRApplication_Utility);
    if(init!=vr::VRInitError_None){snprintf(lastError,sizeof(lastError),"SteamVR: %s",vr::VR_GetVRInitErrorAsEnglishDescription(init));return 0;}
    auto apps=vr::VRApplications();
    if(!apps){vr::VR_Shutdown();snprintf(lastError,sizeof(lastError),"SteamVR applications interface unavailable");return 0;}
    auto added=vr::VRApplicationError_None,set=vr::VRApplicationError_None;
    // The diagnostic uses a unique key with the same persistent registration
    // path, then disables/removes it. OpenVR's bTemporary flag skips the list
    // consumed by RemoveApplicationManifest and cannot test this lifecycle.
    if(enabled||temporary)added=apps->AddApplicationManifest(manifest,false);
    bool installed=apps->IsApplicationInstalled(key);
    if(added==vr::VRApplicationError_None&&installed)set=apps->SetApplicationAutoLaunch(key,enabled!=0);
    bool actual=apps->GetApplicationAutoLaunch(key);
    bool ok=added==vr::VRApplicationError_None && set==vr::VRApplicationError_None && actual==(enabled!=0);
    auto off=vr::VRApplicationError_None,remove=vr::VRApplicationError_None;
    if(temporary){
        if(installed)off=apps->SetApplicationAutoLaunch(key,false);
        if(added==vr::VRApplicationError_None)remove=apps->RemoveApplicationManifest(manifest);
        if(off!=vr::VRApplicationError_None||remove!=vr::VRApplicationError_None)ok=false;
    }
    if(!ok)snprintf(lastError,sizeof(lastError),"SteamVR startup: add=%s, installed=%d, set=%s, read=%d, cleanup=%s/%s",apps->GetApplicationsErrorNameFromEnum(added),installed,apps->GetApplicationsErrorNameFromEnum(set),actual,apps->GetApplicationsErrorNameFromEnum(off),apps->GetApplicationsErrorNameFromEnum(remove));
    vr::VR_Shutdown();return ok?1:0;
}
extern "C" int native_init(const char* manifest,int desktopOnly,unsigned flags) {
    if(flags&8){int result=lifecycle::acquire((flags&16)!=0);if(result!=1){if(!result)snprintf(lastError,sizeof(lastError),"Sylva is already starting or its window is unavailable");return result;}}
    lifecycle::closeToTray=(flags&1)!=0;
    visible=(flags&2)==0;
    hiddenProbe=desktopOnly==5;
    offscreen=desktopOnly==2||desktopOnly==3;
    cpuPanel=desktopOnly==3||desktopOnly==0||desktopOnly==6;
    waitTimer=CreateWaitableTimerExW(nullptr,nullptr,CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,TIMER_ALL_ACCESS);
    frameTimer=CreateWaitableTimerExW(nullptr,nullptr,CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,TIMER_ALL_ACCESS);
    inputWake=CreateEventW(nullptr,FALSE,FALSE,nullptr);
    vrStopEvent=CreateEventW(nullptr,TRUE,FALSE,nullptr);
    CoInitializeEx(nullptr,COINIT_APARTMENTTHREADED);
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    actionManifest=manifest;
    if(!startVr(manifest,desktopOnly))stopVr();
    if(vrReady&&desktopOnly!=4)cpuPanel=true;
    if(!offscreen){
    WNDCLASSW wc{};wc.lpfnWndProc=wndproc;wc.hInstance=GetModuleHandleW(nullptr);wc.lpszClassName=L"SylvaVRPanel";wc.hCursor=LoadCursorW(nullptr,IDC_ARROW);wc.hIcon=lifecycle::appIcon(32);RegisterClassW(&wc);
    RECT rect{0,0,(LONG)W,(LONG)H};DWORD style=WS_OVERLAPPED|WS_CAPTION|WS_SYSMENU|WS_MINIMIZEBOX;AdjustWindowRect(&rect,style,FALSE);
    window=CreateWindowExW(0,wc.lpszClassName,L"Sylva · VR 中文输入法",style,CW_USEDEFAULT,CW_USEDEFAULT,rect.right-rect.left,rect.bottom-rect.top,nullptr,nullptr,wc.hInstance,nullptr);
    if(!window){snprintf(lastError,sizeof(lastError),"CreateWindow failed");return 0;}
    // Avoid routing the physical keyboard through the Windows system IME.
    ImmAssociateContextEx(window,nullptr,0);
    SendMessageW(window,WM_SETICON,ICON_SMALL,(LPARAM)lifecycle::appIcon(GetSystemMetrics(SM_CXSMICON)));
    if(flags&4)lifecycle::addTray(window);
    }
    if(cpuPanel){
        if(!check(software.init(TW,TH,144),"Software panel"))return 0;
        paint=software.target;adapterDescription="CPU WIC / SteamVR SetOverlayRaw";
    }else{
    ComPtr<IDXGIFactory1> factory1; if(!check(CreateDXGIFactory1(IID_PPV_ARGS(&factory1)),"DXGI factory"))return 0;
    ComPtr<IDXGIAdapter1> adapter;
    if(vrReady){int index=-1;vr::VRSystem()->GetDXGIOutputInfo(&index);if(index>=0)factory1->EnumAdapters1(index,&adapter);}
    D3D_FEATURE_LEVEL feature;
    if(!check(D3D11CreateDevice(adapter.Get(),adapter?D3D_DRIVER_TYPE_UNKNOWN:D3D_DRIVER_TYPE_HARDWARE,nullptr,D3D11_CREATE_DEVICE_BGRA_SUPPORT,nullptr,0,D3D11_SDK_VERSION,&device,&feature,&context),"D3D11 device"))return 0;
    ComPtr<IDXGIDevice> dxgi;device.As(&dxgi);
    ComPtr<IDXGIAdapter> actualAdapter;dxgi->GetAdapter(&actualAdapter);
    DXGI_ADAPTER_DESC adapterInfo{};actualAdapter->GetDesc(&adapterInfo);
    char adapterName[256]{};WideCharToMultiByte(CP_UTF8,0,adapterInfo.Description,-1,adapterName,sizeof(adapterName),nullptr,nullptr);adapterDescription=adapterName;
    ComPtr<IDXGIFactory2> factory;actualAdapter->GetParent(IID_PPV_ARGS(&factory));
    DXGI_SWAP_CHAIN_DESC1 sc{};sc.Width=(UINT)W;sc.Height=(UINT)H;sc.Format=DXGI_FORMAT_B8G8R8A8_UNORM;sc.SampleDesc.Count=1;sc.BufferUsage=DXGI_USAGE_RENDER_TARGET_OUTPUT;sc.BufferCount=2;sc.SwapEffect=DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL;sc.Scaling=DXGI_SCALING_STRETCH;
    if(!offscreen){if(!check(factory->CreateSwapChainForHwnd(device.Get(),window,&sc,nullptr,nullptr,&swap),"Swap chain"))return 0;
    factory->MakeWindowAssociation(window,DXGI_MWA_NO_ALT_ENTER);}
    D3D11_TEXTURE2D_DESC td{};td.Width=TW;td.Height=TH;td.MipLevels=1;td.ArraySize=1;td.Format=sc.Format;td.SampleDesc.Count=1;td.Usage=D3D11_USAGE_DEFAULT;td.BindFlags=D3D11_BIND_RENDER_TARGET|D3D11_BIND_SHADER_RESOURCE;
    if(!check(device->CreateTexture2D(&td,nullptr,&texture),"Overlay texture"))return 0;
    ComPtr<ID2D1Factory1> d2d;D2D1_FACTORY_OPTIONS opts{};
    if(!check(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED,opts,d2d.GetAddressOf()),"D2D factory"))return 0;
    ComPtr<ID2D1Device> d2device;if(!check(d2d->CreateDevice(dxgi.Get(),&d2device),"D2D device"))return 0;
    if(!check(d2device->CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE,&draw),"D2D context"))return 0;
    ComPtr<IDXGISurface> surface;texture.As(&surface);
    auto props=D2D1::BitmapProperties1(D2D1_BITMAP_OPTIONS_TARGET|D2D1_BITMAP_OPTIONS_CANNOT_DRAW,D2D1::PixelFormat(sc.Format,D2D1_ALPHA_MODE_PREMULTIPLIED),144,144);
    // The panel is also sampled by the desktop preview.
    props.bitmapOptions=D2D1_BITMAP_OPTIONS_TARGET;
    if(!check(draw->CreateBitmapFromDxgiSurface(surface.Get(),&props,&target),"Panel bitmap"))return 0;
    if(!offscreen){ComPtr<IDXGISurface> back;if(!check(swap->GetBuffer(0,IID_PPV_ARGS(&back)),"Swap surface"))return 0;props.dpiX=props.dpiY=96;
    props.bitmapOptions=D2D1_BITMAP_OPTIONS_TARGET|D2D1_BITMAP_OPTIONS_CANNOT_DRAW;props.pixelFormat.alphaMode=D2D1_ALPHA_MODE_IGNORE;
    if(!check(draw->CreateBitmapFromDxgiSurface(back.Get(),&props,&desktopTarget),"Desktop bitmap"))return 0;}
    draw.As(&paint);
    }
    paint->CreateSolidColorBrush(D2D1::ColorF(0,0,0,1),&brush);paint->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    if(!check(DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED,__uuidof(IDWriteFactory),reinterpret_cast<IUnknown**>(fonts.GetAddressOf())),"DirectWrite"))return 0;
    if(!offscreen&&!hiddenProbe&&(!(flags&2)||!lifecycle::trayAdded)){lifecycle::show(window);UpdateWindow(window);}
    if(vrReady)vrThread=std::thread(vrLoop);
    return 1;
}
extern "C" void native_lifecycle_settings(int close){lifecycle::closeToTray=close!=0;}
extern "C" int native_desktop_visible(){return window&&IsWindowVisible(window)&&!IsIconic(window);}
extern "C" int native_attach_vr(){
    if(vrReady)return 1;
    if(!cpuPanel||offscreen)return 0;
    panelWidth=requestedWidth.load();
    if(!startVr(actionManifest.c_str(),0)){stopVr();return 0;}
    lastError[0]=0;rawDirty=true;vrThread=std::thread(vrLoop);return 1;
}
extern "C" int native_vr_ready(){return vrReady;}
extern "C" int native_visible(){return visibleToUi.load();}
extern "C" void native_vr_settings(float width,int block,int haptics,int center){requestedWidth.store(width);requestedBlock.store(block!=0);requestedHaptics.store(haptics!=0);if(!haptics)feedbackHands.store(0);if(center)recenterRequested.store(true);}
static void pollPanelEvents(){
    if(!vrReady)return;
    // Polling also retires the runtime's raw-image memory. Only one upload may
    // be outstanding; changes while it loads replace the next pending image.
    vr::VREvent_t imageEvent{};
    for(int slot=0;slot<2;slot++)if(panelSlots[slot])while(vr::VROverlay()->PollNextOverlayEvent(panelSlots[slot],&imageEvent,sizeof(imageEvent))){
        if(imageEvent.eventType==vr::VREvent_ImageLoaded&&cpuPanel){
            auto start=rawSubmittedAt.load();
            if(rawSwap.loading()==slot&&rawSwap.ready()<0&&loadedImageSlot<0){
                {std::lock_guard<std::mutex> lock(metricsMutex);rawLoadTimes.add(double(nowNanoseconds()-start)/1000000.);}
                rawLoaded.fetch_add(1);loadedImageSlot=slot;if(inputWake)SetEvent(inputWake);
            }
        }else if(imageEvent.eventType==vr::VREvent_ImageFailed&&cpuPanel){rawFailed.store(true);rawSwap.failed(slot);if(inputWake)SetEvent(inputWake);}
    }
    // Texture queries can block inside the runtime. Keep them off the pointer
    // thread, and only inspect the front during explicit hidden diagnostics.
    if(loadedImageSlot>=0&&panelTextureAvailable(loadedImageSlot)){rawSwap.loaded(loadedImageSlot);loadedImageSlot=-1;}
    if(hiddenProbe&&rawSwap.loading()>=0){int front=rawSwap.front();if(front>=0){frontChecks++;if(!panelTextureAvailable(front))frontMissing++;}}
}
static void vrPoll(std::chrono::steady_clock::time_point now){
    if(!vrReady)return;
    vr::VREvent_t imageEvent{};
    for(auto cursor:cursors)if(cursor)while(vr::VROverlay()->PollNextOverlayEvent(cursor,&imageEvent,sizeof(imageEvent))){}
    for(int i=0;i<2;i++)for(int part=0;part<3;part++)if(keyFeedback[i][part])while(vr::VROverlay()->PollNextOverlayEvent(keyFeedback[i][part],&imageEvent,sizeof(imageEvent))){if(imageEvent.eventType==vr::VREvent_ImageLoaded)keyFeedbackLoaded[i][part]=true;else if(imageEvent.eventType==vr::VREvent_ImageFailed)keyFeedbackLoaded[i][part]=false;}
    vr::VREvent_t event{};while(vr::VRSystem()->PollNextEvent(&event,sizeof(event))){
        if(event.eventType==vr::VREvent_Quit){vr::VRSystem()->AcknowledgeQuit_Exiting();queueEvent({1,0,0,0,0});return;}
        if(event.eventType==vr::VREvent_TrackedDeviceActivated || event.eventType==vr::VREvent_TrackedDeviceDeactivated || event.eventType==vr::VREvent_TrackedDeviceUpdated)
            for(auto& tip:controllerTips)tip.index=vr::k_unTrackedDeviceIndexInvalid;
    }
    if(hiddenProbe)return;
    bool dashboard=dashboardVisible=vr::VROverlay()->IsDashboardVisible();
    vr::TrackedDevicePose_t poses[vr::k_unMaxTrackedDeviceCount]{};
    vr::VRSystem()->GetDeviceToAbsoluteTrackingPose(vr::TrackingUniverseStanding,predictionSeconds,poses,vr::k_unMaxTrackedDeviceCount);
    if(visible&&!placed)recenter();
    bool valid[2]{};vr::HmdMatrix34_t handPose[2]{},rays[2]{};sylva::PanelHit hits[2]{};
    for(int i=0;i<2;i++){
        auto index=vr::VRSystem()->GetTrackedDeviceIndexForControllerRole(i?vr::TrackedControllerRole_RightHand:vr::TrackedControllerRole_LeftHand);
        valid[i]=index<vr::k_unMaxTrackedDeviceCount && poses[index].bPoseIsValid && poses[index].bDeviceIsConnected;
        if(!valid[i]||!visible||dashboard||!placed)continue;
        handPose[i]=poses[index].mDeviceToAbsoluteTracking;
        auto& cached=controllerTips[i];
        // Quest/PICO tip geometry is static. Avoid two render-model IPC calls per hand per frame.
        if(cached.index!=index){
            cached.index=index;char model[256]{};vr::VRSystem()->GetStringTrackedDeviceProperty(index,vr::Prop_RenderModelName_String,model,sizeof(model));
            vr::RenderModel_ControllerMode_State_t mode{};vr::RenderModel_ComponentState_t tip{};
            cached.valid=vr::VRRenderModels() && vr::VRRenderModels()->GetComponentStateForDevicePath(model,"tip",hands[i],&mode,&tip);
            if(cached.valid)cached.transform=tip.mTrackingToComponentLocal;
        }
        rays[i]=cached.valid?mul(handPose[i],cached.transform):handPose[i];
        hits[i]=sylva::intersectPanel(panel,panelWidth,rays[i]);
    }
    // OpenVR needs the priority before it supplies this frame's button edges.
    // A press that began on the panel therefore protects its release frame too.
    const bool ownsInput = (valid[0] && panelButtons[0].ownsInput(hits[0].hit)) ||
                           (valid[1] && panelButtons[1].ownsInput(hits[1].hit)) ||
                           (grabHand >= 0 && valid[grabHand]);
    vr::VRActiveActionSet_t set{};set.ulActionSet=actions;
    if (blockInput && visible && placed && !dashboard && ownsInput)
        set.nPriority = vr::k_nActionSetOverlayGlobalPriorityMax;
    auto update=vr::VRInput()->UpdateActionState(&set,sizeof(set),1);
    if(update!=vr::VRInputError_None){resetHands();return;}
    vr::InputDigitalActionData_t toggle{};
    vr::VRInput()->GetDigitalActionData(toggleAction,&toggle,sizeof(toggle),vr::k_ulInvalidInputValueHandle);
    if(toggle.bActive && toggle.bChanged && toggle.bState){
        visible=!visible;
        resetHands();
        if(visible){
            // Never show the dragged pose while waiting for valid HMD tracking.
            placed=false;
            recenter();
        }
        queueEvent({8,0,0,0,visible?1u:0u});
        return; // This frame's rays were calculated before the toggle.
    }
    vr::InputDigitalActionData_t clicks[2]{},grabs[2]{};bool active[2]{};
    for(int i=0;i<2;i++){
        auto& c=clicks[i];auto& g=grabs[i];
        vr::VRInput()->GetDigitalActionData(clickAction,&c,sizeof(c),hands[i]);vr::VRInput()->GetDigitalActionData(grabAction,&g,sizeof(g),hands[i]);
        if(!valid[i]||!visible||dashboard||!placed||!c.bActive||!g.bActive){if(armed[i])queueEvent({7,i,0,0,0});armed[i]=false;panelButtons[i]={};if(grabHand==i)grabHand=-1;showCursor(i,false);continue;}
        if(!armed[i]){if(!c.bState&&!g.bState)armed[i]=true;continue;}
        active[i]=true;
        if (panelButtons[i].startGrab(g.bState, hits[i].hit && grabHand < 0, now)) {
            grabHand = i;
            grabOffset = mul(inverse(handPose[i]), panel);
            for (int j = 0; j < 2; ++j) queueEvent({7,j,0,0,0});
        }
        if (grabHand == i) {
            if (g.bState) {
                panel = mul(handPose[i], grabOffset);
                panelTransform();
            } else {
                grabHand = -1;
                if (sylva::settlePanel(panel)) panelTransform();
            }
        }
    }
    // Recompute both hands after moving the panel; neither cursor uses last frame's plane.
    for(int i=0;i<2;i++){
        if(!active[i])continue;
        auto hit=sylva::intersectPanel(panel,panelWidth,rays[i]);
        float x=hit.x,y=hit.y;
        const auto edge = panelButtons[i].trigger(clicks[i].bState, hit.hit);
        updateKeyFeedback(i,hit,edge == sylva::TriggerEdge::press);
        queueEvent({2,i,x,y,0});
        if (grabHand < 0) {
            if (edge == sylva::TriggerEdge::press) queueEvent({3,i,x,y,0});
            if (edge == sylva::TriggerEdge::release) queueEvent({4,i,x,y,0});
        }
        if(hit.hit&&cursors[i]){auto cursor=panel;for(int j=0;j<3;j++)cursor.m[j][3]=hit.point.v[j]+panel.m[j][2]*.0005f;vr::VROverlay()->SetOverlayTransformAbsolute(cursors[i],vr::TrackingUniverseStanding,&cursor);}
        showCursor(i,hit.hit);
    }
}
static void vrLoop(){
    auto previous=std::chrono::steady_clock::now();
    while(!vrStop.load()) {
        auto begin=std::chrono::steady_clock::now();
        float width=requestedWidth.load();
        if(width!=panelWidth){panelWidth=width;panelWidthSetting();}
        blockInput=requestedBlock.load();
        if(recenterRequested.exchange(false)){recenter();resetHands();}
        vrPoll(begin);
        auto feedback=feedbackHands.exchange(0);
        if(requestedHaptics.load()&&visible&&!dashboardVisible&&hapticAction)for(int i=0;i<2;i++)if(feedback&(1u<<i))vr::VRInput()->TriggerHapticVibrationAction(hapticAction,0,.018f,120.f,.25f,hands[i]);
        presentPanel(visible&&placed);
        visibleToUi.store(textureReady.load()&&visible&&placed&&!dashboardVisible);
        auto end=std::chrono::steady_clock::now();
        {
            std::lock_guard<std::mutex> lock(metricsMutex);
            if(visible&&!dashboardVisible&&pollTimes.size()<4096){
                pollTimes.push_back(std::chrono::duration<double,std::milli>(end-begin).count());
                frameTimes.push_back(std::chrono::duration<double,std::milli>(begin-previous).count());
            }
        }
        previous=begin;
        if(visible&&!dashboardVisible){
            auto start=std::chrono::steady_clock::now();
            auto result=vr::VROverlay()->WaitFrameSync(frameTimeout);
            {std::lock_guard<std::mutex> lock(metricsMutex);waitTimes.add(sylva::milliseconds(start));if(result==vr::VROverlayError_TimedOut)waitTimeouts++;else if(result!=vr::VROverlayError_None)waitErrors++;}
            if(result!=vr::VROverlayError_None){
                auto remaining=sylva::remainingTicks(displayHz,sylva::milliseconds(begin));
                if(remaining>0){
                    if(frameTimer){LARGE_INTEGER due{};due.QuadPart=-remaining;SetWaitableTimer(frameTimer,&due,0,nullptr,nullptr,FALSE);HANDLE handles[]{vrStopEvent,frameTimer};WaitForMultipleObjects(2,handles,FALSE,frameTimeout+1);}
                    else WaitForSingleObject(vrStopEvent,(DWORD)((remaining+9999)/10000));
                }
            }
        }else WaitForSingleObject(vrStopEvent,33);
    }
}
extern "C" void native_pump(){MSG m;while(PeekMessageW(&m,nullptr,0,0,PM_REMOVE)){TranslateMessage(&m);DispatchMessageW(&m);}pollPanelEvents();}
extern "C" int native_event(Event*out){std::lock_guard<std::mutex> lock(eventMutex);if(events.empty())return 0;*out=events.front();events.pop_front();return 1;}
extern "C" void native_wake(){if(inputWake)SetEvent(inputWake);}
extern "C" void native_feedback(unsigned hand){if(hand<2&&requestedHaptics.load())feedbackHands.fetch_or(1u<<hand);}
extern "C" void native_key_regions(const KeyRegion* regions,unsigned count){if(!vrReady)return;std::lock_guard<std::mutex> lock(regionsMutex);keyRegions.assign(regions,regions+count);}
extern "C" void native_wait(unsigned ms){
    // VR motion and completed IME results wake the UI immediately. No fixed delay
    // is added to the separately paced pointer loop.
    HANDLE handles[2]{inputWake,waitTimer};DWORD count=inputWake?1:0;
    if(waitTimer){LARGE_INTEGER due{};due.QuadPart=-(LONGLONG)ms*10000;SetWaitableTimer(waitTimer,&due,0,nullptr,nullptr,FALSE);handles[count++]=waitTimer;}
    MsgWaitForMultipleObjectsEx(count,handles,ms+1,QS_ALLINPUT,MWMO_INPUTAVAILABLE);
}
extern "C" unsigned native_vr_metrics(double* result){
    std::vector<double> polls,frames;
    {std::lock_guard<std::mutex> lock(metricsMutex);polls=pollTimes;frames=frameTimes;}
    if(polls.empty())return 0;
    auto summarize=[](std::vector<double> values,double* out){std::sort(values.begin(),values.end());out[0]=values[values.size()/2];out[1]=values[values.size()*95/100];out[2]=values.back();};
    summarize(polls,result);summarize(frames,result+3);return (unsigned)polls.size();
}
extern "C" void native_graphics_metrics(double* out){graphicsMetrics.report(out,context.Get());}
extern "C" void native_metrics_reset(){graphicsMetrics.reset();prepareTimes.values.clear();}
extern "C" void native_metrics_stop(){graphicsMetrics.enabled=false;}
extern "C" void native_text_cache(int enabled){textCache=enabled!=0;layouts.clear();layoutHits=layoutMisses=0;}
extern "C" void native_cache_metrics(double* out){out[0]=double(layoutHits);out[1]=double(layoutMisses);out[2]=double(layouts.size());}
extern "C" const char* native_adapter(){return adapterDescription.c_str();}
extern "C" int native_cpu_panel(){return cpuPanel?1:0;}
extern "C" void native_incremental(int enabled){damage.incremental=enabled!=0;}
extern "C" void native_prepare_metrics(double* out){prepareTimes.summarize(out);}
extern "C" void native_raw_metrics(double* out){std::lock_guard<std::mutex> lock(metricsMutex);rawLoadTimes.summarize(out);out[4]=double(rawSubmitted);out[5]=double(rawLoaded.load());out[6]=double(rawCoalesced);out[7]=rawSwap.loading()>=0?1:0;out[8]=rawFailed.load()?1:0;out[9]=double(feedbackLoaded(0))+double(feedbackLoaded(1));out[10]=double(rawSwaps.load());out[11]=double(frontChecks);out[12]=double(frontMissing);out[13]=hiddenProbe?1:0;out[14]=rawSwap.loading()>=0?double(nowNanoseconds()-rawSubmittedAt.load())/1000000.:0;}
extern "C" void native_bgra_to_rgba(const unsigned char* src,unsigned stride,unsigned width,unsigned height,unsigned char* dst){sylva::bgraToRgba(src,stride,width,height,dst);}
extern "C" void* native_swap_new(){return new sylva::RawSwap;}
extern "C" void native_swap_delete(void* p){delete static_cast<sylva::RawSwap*>(p);}
extern "C" int native_swap_step(void* p,int operation,int slot){auto&s=*static_cast<sylva::RawSwap*>(p);switch(operation){case 0:return s.begin();case 1:return s.loaded(slot);case 2:return s.presented(slot);case 3:s.failed(slot);return 0;case 4:return s.front();case 5:return s.loading();default:return s.ready();}}
extern "C" const char* native_runtime_version(){return runtimeVersion.c_str();}
extern "C" void native_wait_metrics(double* out){std::lock_guard<std::mutex> lock(metricsMutex);waitTimes.summarize(out);out[4]=waitTimeouts;out[5]=waitErrors;out[6]=displayHz;}
extern "C" void native_process_metrics(double* out){
    FILETIME create{},exit{},kernel{},user{};ULARGE_INTEGER k{},u{};
    GetProcessTimes(GetCurrentProcess(),&create,&exit,&kernel,&user);k.LowPart=kernel.dwLowDateTime;k.HighPart=kernel.dwHighDateTime;u.LowPart=user.dwLowDateTime;u.HighPart=user.dwHighDateTime;
    PROCESS_MEMORY_COUNTERS_EX memory{};memory.cb=sizeof(memory);GetProcessMemoryInfo(GetCurrentProcess(),reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&memory),sizeof(memory));
    out[0]=double(k.QuadPart+u.QuadPart)/10000000.;out[1]=double(memory.WorkingSetSize)/1048576.;out[2]=double(memory.PrivateUsage)/1048576.;
}
extern "C" long long native_frame_remaining(double hz,double elapsedMs){return sylva::remainingTicks(hz,elapsedMs);}
// This snapshot belongs to SteamVR's scene application, not the keyboard GPU.
// Some runtimes deny timing data to overlays; a zero count means unavailable.
extern "C" unsigned native_scene_metrics(double* out){
    if(!vrReady||!vr::VRCompositor())return 0;
    vr::Compositor_FrameTiming frames[128]{};for(auto& f:frames)f.m_nSize=sizeof(f);
    unsigned count=vr::VRCompositor()->GetFrameTimings(frames,128);if(count>128)count=128;
    sylva::Samples sceneGpu,compositorGpu,interval,submit;
    for(unsigned i=0;i<count;i++){sceneGpu.add(frames[i].m_flPreSubmitGpuMs);compositorGpu.add(frames[i].m_flCompositorRenderGpuMs);interval.add(frames[i].m_flClientFrameIntervalMs);submit.add(frames[i].m_flSubmitFrameMs);}
    sceneGpu.summarize(out);compositorGpu.summarize(out+4);interval.summarize(out+8);submit.summarize(out+12);
    vr::Compositor_CumulativeStats stats{};vr::VRCompositor()->GetCumulativeStats(&stats,sizeof(stats));
    out[16]=stats.m_nPid;out[17]=stats.m_nNumFramePresents;out[18]=stats.m_nNumDroppedFrames;out[19]=stats.m_nNumReprojectedFrames;
    out[20]=vr::VRCompositor()->GetCurrentSceneFocusProcess();return count;
}
extern "C" void native_begin(unsigned bg){
    if(cpuPanel){graphicsMetrics.begin=sylva::Clock::now();damage.begin(bg);return;}
    else{graphicsMetrics.start(device.Get(),context.Get());draw->SetTarget(target.Get());}
    paint->SetDpi(144,144);paint->BeginDraw();paint->Clear(color(bg));
}
extern "C" void native_rect(float x,float y,float w,float h,float radius,unsigned fill,unsigned border){
    if(cpuPanel&&!replaying){damage.current.push_back({false,x,y,w,h,radius,fill,border,0,{}});return;}
    auto r=D2D1::RoundedRect(D2D1::RectF(x,y,x+w,y+h),radius,radius);brush->SetColor(color(fill));paint->FillRoundedRectangle(r,brush.Get());if(border>>24){brush->SetColor(color(border));paint->DrawRoundedRectangle(r,brush.Get(),1);}
}
extern "C" void native_icon(unsigned id,float x,float y,float size,unsigned fill){
    if(id>=5)return;
    if(cpuPanel&&!replaying){sylva::PaintCommand c;c.x=x;c.y=y;c.w=c.h=size;c.fill=fill;c.icon=int(id);damage.current.push_back(std::move(c));return;}
    ComPtr<ID2D1Factory> factory;paint->GetFactory(&factory);
    if(!iconStroke){
        auto style=D2D1::StrokeStyleProperties();style.startCap=style.endCap=style.dashCap=D2D1_CAP_STYLE_ROUND;style.lineJoin=D2D1_LINE_JOIN_ROUND;
        if(FAILED(factory->CreateStrokeStyle(style,nullptr,0,&iconStroke)))return;
    }
    if(!iconGeometry[id]){
        ComPtr<ID2D1PathGeometry> geometry;ComPtr<ID2D1GeometrySink> sink;
        if(FAILED(factory->CreatePathGeometry(&geometry))||FAILED(geometry->Open(&sink)))return;
        bool open=false;
        for(unsigned i=0;i<svgIcons[id].count;i++){
            auto& c=svgIcons[id].commands[i];auto p=D2D1::Point2F(c.v[0],c.v[1]);
            if(c.verb=='M'){if(open)sink->EndFigure(D2D1_FIGURE_END_OPEN);sink->BeginFigure(p,D2D1_FIGURE_BEGIN_HOLLOW);open=true;}
            else if(c.verb=='L')sink->AddLine(p);
            else if(c.verb=='C')sink->AddBezier(D2D1::BezierSegment(p,D2D1::Point2F(c.v[2],c.v[3]),D2D1::Point2F(c.v[4],c.v[5])));
            else if(c.verb=='Z'){sink->EndFigure(D2D1_FIGURE_END_CLOSED);open=false;}
        }
        if(open)sink->EndFigure(D2D1_FIGURE_END_OPEN);
        if(FAILED(sink->Close()))return;iconGeometry[id]=geometry;
    }
    D2D1_MATRIX_3X2_F previous{};paint->GetTransform(&previous);
    paint->SetTransform(D2D1::Matrix3x2F::Scale(size/24.f,size/24.f)*D2D1::Matrix3x2F::Translation(x,y)*previous);
    brush->SetColor(color(fill));paint->DrawGeometry(iconGeometry[id].Get(),brush.Get(),1.8f,iconStroke.Get());paint->SetTransform(previous);
}
extern "C" unsigned native_copy_text(const unsigned short* text,unsigned len){
    auto memory=GlobalAlloc(GMEM_MOVEABLE,(size_t(len)+1)*sizeof(wchar_t));if(!memory)return ERROR_NOT_ENOUGH_MEMORY;
    auto output=static_cast<wchar_t*>(GlobalLock(memory));if(!output){GlobalFree(memory);return ERROR_NOT_ENOUGH_MEMORY;}
    memcpy(output,text,size_t(len)*sizeof(wchar_t));output[len]=0;GlobalUnlock(memory);
    if(!OpenClipboard(window)){auto e=GetLastError();GlobalFree(memory);return e?e:ERROR_ACCESS_DENIED;}
    unsigned error=0;
    if(!EmptyClipboard()||!SetClipboardData(CF_UNICODETEXT,memory)){error=GetLastError();if(!error)error=ERROR_ACCESS_DENIED;GlobalFree(memory);}
    // Ownership transfers to Windows only after SetClipboardData succeeds.
    CloseClipboard();return error;
}
extern "C" int native_neural_background(){return SetThreadPriority(GetCurrentThread(),THREAD_PRIORITY_BELOW_NORMAL)?1:0;}
// The UI thread uses one shaped line for painting, underlining and both kinds
// of pointer. Retain the horizontal viewport while the caret stays in view.
struct DraftLineLayout {
    ComPtr<IDWriteTextLayout> layout;
    std::wstring label;
    int start=-1;
    unsigned length=0;
    float scroll=0, caretX=0;

    bool prepare(const sylva::PaintCommand& cmd) {
        if(layout && label==cmd.label && start==cmd.preeditStart && length==cmd.preeditLength)
            return true;
        layout.Reset();
        auto& format=formats[int(cmd.size*10)];
        if(!format && FAILED(fonts->CreateTextFormat(L"Microsoft YaHei UI",nullptr,DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,DWRITE_FONT_STRETCH_NORMAL,cmd.size,L"zh-CN",&format)))return false;
        format->SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        format->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        format->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        ComPtr<IDWriteTextLayout> next;
        if(FAILED(fonts->CreateTextLayout(cmd.label.data(),UINT32(cmd.label.size()),format.Get(),250000.f,cmd.h,&next)))return false;
        if(cmd.preeditLength)next->SetUnderline(TRUE,{UINT32(cmd.preeditStart),cmd.preeditLength});
        UINT32 end=UINT32(cmd.preeditStart)+cmd.preeditLength;
        bool trailing=end>0 && (cmd.preeditLength || end==cmd.label.size());
        FLOAT y;
        DWRITE_HIT_TEST_METRICS hit{};
        DWRITE_TEXT_METRICS metrics{};
        if(FAILED(next->HitTestTextPosition(trailing?end-1:end,trailing,&caretX,&y,&hit))
            || FAILED(next->GetMetrics(&metrics)))return false;
        float margin=cmd.w*.20f;
        if(caretX<scroll+margin)scroll=caretX-margin;
        else if(caretX>scroll+cmd.w-margin)scroll=caretX+margin-cmd.w;
        scroll=std::clamp(scroll,0.f,std::max(0.f,metrics.widthIncludingTrailingWhitespace+2.f-cmd.w));
        label=cmd.label;start=cmd.preeditStart;length=cmd.preeditLength;
        layout=std::move(next);
        return true;
    }
};
static DraftLineLayout draftLine;
static void drawDraftLine(const sylva::PaintCommand& cmd) {
    if(!draftLine.prepare(cmd))return;
    const auto origin=D2D1::Point2F(cmd.x-draftLine.scroll,cmd.y);
    paint->PushAxisAlignedClip(D2D1::RectF(cmd.x,cmd.y,cmd.x+cmd.w,cmd.y+cmd.h),D2D1_ANTIALIAS_MODE_ALIASED);
    brush->SetColor(color(cmd.fill));
    paint->DrawTextLayout(origin,draftLine.layout.Get(),brush.Get());
    float x=std::clamp(origin.x+draftLine.caretX,cmd.x,cmd.x+cmd.w-2.f);
    paint->FillRectangle(D2D1::RectF(x,cmd.y+6.f,x+2.f,cmd.y+cmd.h-6.f),brush.Get());
    paint->PopAxisAlignedClip();
}
extern "C" void native_draft(const unsigned short* text,unsigned len,unsigned start,unsigned length,
                              float x,float y,float w,float h,float size,unsigned fill){
    sylva::PaintCommand cmd;
    cmd.label.assign(reinterpret_cast<const wchar_t*>(text),len);
    cmd.preeditStart=int(start);cmd.preeditLength=length;
    cmd.x=x;cmd.y=y;cmd.w=w;cmd.h=h;cmd.size=size;cmd.fill=fill;
    if(cpuPanel){damage.current.push_back(std::move(cmd));return;}
    drawDraftLine(cmd);
}
extern "C" int native_draft_hit(float localX,float localY) {
    if(!draftLine.layout)return -1;
    BOOL trailing,inside;
    DWRITE_HIT_TEST_METRICS hit{};
    if(FAILED(draftLine.layout->HitTestPoint(localX+draftLine.scroll,localY,&trailing,&inside,&hit)))return -1;
    return int(hit.textPosition+(trailing?hit.length:0));
}
extern "C" void native_text(const unsigned short* text,unsigned len,float x,float y,float w,float h,float size,unsigned fill,int align){
    if(cpuPanel&&!replaying){damage.current.push_back({true,x,y,w,h,size,fill,0,align,std::wstring(reinterpret_cast<const wchar_t*>(text),len)});return;}
    int key=int(size*10);auto &fmt=formats[key];if(!fmt){fonts->CreateTextFormat(L"Microsoft YaHei UI",nullptr,DWRITE_FONT_WEIGHT_NORMAL,DWRITE_FONT_STYLE_NORMAL,DWRITE_FONT_STRETCH_NORMAL,size,L"zh-CN",&fmt);fmt->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);fmt->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);}
    brush->SetColor(color(fill));
    if(!textCache){
        fmt->SetTextAlignment(align==1?DWRITE_TEXT_ALIGNMENT_CENTER:align==2?DWRITE_TEXT_ALIGNMENT_TRAILING:DWRITE_TEXT_ALIGNMENT_LEADING);
        paint->DrawText(reinterpret_cast<const wchar_t*>(text),len,fmt.Get(),D2D1::RectF(x,y,x+w,y+h),brush.Get(),D2D1_DRAW_TEXT_OPTIONS_CLIP);return;
    }
    LayoutKey layoutKey{std::wstring(reinterpret_cast<const wchar_t*>(text),len),key,w,h,align};
    auto found=layouts.find(layoutKey);
    if(found==layouts.end()){
        // LRU bound includes dynamic candidate/draft strings; static key labels stay hot.
        if(layouts.size()>=256){auto oldest=std::min_element(layouts.begin(),layouts.end(),[](const auto&a,const auto&b){return a.second.used<b.second.used;});layouts.erase(oldest);}
        CachedLayout entry;fmt->SetTextAlignment(align==1?DWRITE_TEXT_ALIGNMENT_CENTER:align==2?DWRITE_TEXT_ALIGNMENT_TRAILING:DWRITE_TEXT_ALIGNMENT_LEADING);
        if(FAILED(fonts->CreateTextLayout(reinterpret_cast<const wchar_t*>(text),len,fmt.Get(),w,h,&entry.value)))return;
        found=layouts.emplace(std::move(layoutKey),std::move(entry)).first;layoutMisses++;
    }else layoutHits++;
    found->second.used=++layoutUse;
    paint->DrawTextLayout(D2D1::Point2F(x,y),found->second.value.Get(),brush.Get(),D2D1_DRAW_TEXT_OPTIONS_CLIP);
}
extern "C" float native_text_width(const unsigned short* text,unsigned len,float size){
    if(!fonts)return -1.f;if(!len)return 0.f;
    auto key=std::make_pair(std::wstring(reinterpret_cast<const wchar_t*>(text),len),int(size*10));
    auto found=textWidths.find(key);if(found!=textWidths.end()){found->second.used=++widthUse;return found->second.width;}
    auto& fmt=formats[key.second];
    if(!fmt){if(FAILED(fonts->CreateTextFormat(L"Microsoft YaHei UI",nullptr,DWRITE_FONT_WEIGHT_NORMAL,DWRITE_FONT_STYLE_NORMAL,DWRITE_FONT_STRETCH_NORMAL,size,L"zh-CN",&fmt)))return -1.f;fmt->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);fmt->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);}
    fmt->SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
    ComPtr<IDWriteTextLayout> layout;DWRITE_TEXT_METRICS metrics{};
    if(FAILED(fonts->CreateTextLayout(key.first.data(),len,fmt.Get(),100000.f,64.f,&layout))||FAILED(layout->GetMetrics(&metrics)))return -1.f;
    if(textWidths.size()>=512){auto oldest=std::min_element(textWidths.begin(),textWidths.end(),[](const auto&a,const auto&b){return a.second.used<b.second.used;});textWidths.erase(oldest);}
    textWidths.emplace(std::move(key),WidthEntry{metrics.widthIncludingTrailingWhitespace,++widthUse});return metrics.widthIncludingTrailingWhitespace;
}
static bool publishRaw(){
    if(rawFailed.load()){snprintf(lastError,sizeof(lastError),"SteamVR failed to load the panel image");return false;}
    if(!vrReady||!rawDirty)return true;
    int slot=rawSwap.begin();if(slot<0)return true;
    rawSubmittedAt.store(nowNanoseconds());
    auto stage=sylva::Clock::now();
    auto error=vr::VROverlay()->SetOverlayRaw(panelSlots[slot],software.rgba.data(),TW,TH,4);
    graphicsMetrics.add(sylva::GraphicsMetrics::Submit,sylva::milliseconds(stage));
    if(error!=vr::VROverlayError_None){rawSwap.failed(slot);snprintf(lastError,sizeof(lastError),"SetOverlayRaw: %d",error);return false;}
    rawDirty=false;rawSubmitted++;return true;
}
extern "C" int native_present(){
    D2D1_RECT_F changed{};
    if(cpuPanel){
        changed=damage.bounds();
        if(changed.right<=changed.left||changed.bottom<=changed.top){damage.commit();return publishRaw()?1:0;}
        paint->SetDpi(144,144);paint->BeginDraw();paint->PushAxisAlignedClip(changed,D2D1_ANTIALIAS_MODE_ALIASED);paint->Clear(color(damage.background));
        replaying=true;
        for(const auto& c:damage.current){
            if(c.x+c.w+2<changed.left||c.y+c.h+2<changed.top||c.x-2>changed.right||c.y-2>changed.bottom)continue;
            if(c.icon>=0)native_icon(unsigned(c.icon),c.x,c.y,c.w,c.fill);
            else if(c.preeditStart>=0)drawDraftLine(c);
            else if(c.text)native_text(reinterpret_cast<const unsigned short*>(c.label.data()),(unsigned)c.label.size(),c.x,c.y,c.w,c.h,c.size,c.fill,c.align);
            else native_rect(c.x,c.y,c.w,c.h,c.size,c.fill,c.border);
        }
        replaying=false;paint->PopAxisAlignedClip();
    }
    graphicsMetrics.add(sylva::GraphicsMetrics::Record,sylva::milliseconds(graphicsMetrics.begin));
    auto stage=sylva::Clock::now();
    if(!check(paint->EndDraw(),"Draw panel"))return 0;
    graphicsMetrics.add(sylva::GraphicsMetrics::EndDraw,sylva::milliseconds(stage));
    if(cpuPanel){
        stage=sylva::Clock::now();if(!check(software.prepare((unsigned)std::lround(changed.left*1.5f),(unsigned)std::lround(changed.top*1.5f),(unsigned)std::lround(changed.right*1.5f),(unsigned)std::lround(changed.bottom*1.5f)),"Prepare raw panel"))return 0;
        damage.commit();
        if(graphicsMetrics.enabled)prepareTimes.add(sylva::milliseconds(stage));
        if(rawDirty)rawCoalesced++;
        rawDirty=true;if(!publishRaw())return 0;
        desktopDirty=true;return 1;
    }
    graphicsMetrics.end(context.Get());draw->SetTarget(nullptr);stage=sylva::Clock::now();context->Flush();
    graphicsMetrics.add(sylva::GraphicsMetrics::Flush,sylva::milliseconds(stage));stage=sylva::Clock::now();
    if(vrReady){vr::Texture_t t{texture.Get(),vr::TextureType_DirectX,vr::ColorSpace_Gamma};auto e=vr::VROverlay()->SetOverlayTexture(overlay,&t);if(e!=vr::VROverlayError_None){snprintf(lastError,sizeof(lastError),"SetOverlayTexture: %d",e);return 0;}textureReady.store(true);}
    if(vrReady)graphicsMetrics.add(sylva::GraphicsMetrics::Submit,sylva::milliseconds(stage));
    desktopDirty=true;
    return 1;
}
// Desktop mirroring never holds up the headset submit. Busy presents are retried,
// and a hidden/minimized window never copies the panel to a second display.
extern "C" int native_mirror(){
    if(cpuPanel&&!publishRaw())return 0;
    if(offscreen||hiddenProbe||!desktopDirty||!IsWindowVisible(window)||IsIconic(window))return 1;
    auto now=std::chrono::steady_clock::now();
    if(vrReady&&now-lastDesktopPresent<std::chrono::milliseconds(33))return 1;
    lastDesktopPresent=now;
    if(cpuPanel){if(!check(software.mirror(window),"Mirror raw panel"))return 0;desktopDirty=false;graphicsMetrics.add(sylva::GraphicsMetrics::Mirror,sylva::milliseconds(now));return 1;}
    draw->SetTarget(desktopTarget.Get());draw->SetDpi(96,96);draw->BeginDraw();
    draw->DrawBitmap(target.Get(),D2D1::RectF(0,0,W,H),1,D2D1_INTERPOLATION_MODE_LINEAR);
    if(!check(draw->EndDraw(),"Draw desktop"))return 0;
    draw->SetTarget(nullptr);
    HRESULT hr=swap->Present(0,DXGI_PRESENT_DO_NOT_WAIT);
    graphicsMetrics.add(sylva::GraphicsMetrics::Mirror,sylva::milliseconds(now));
    if(hr==DXGI_ERROR_WAS_STILL_DRAWING)return 1;
    if(!check(hr,"Present"))return 0;
    desktopDirty=false;return 1;
}
// Geometry regression tests use the exact production intersection function.
extern "C" int native_panel_hit(const vr::HmdMatrix34_t* p,float width,const vr::HmdMatrix34_t* ray,float* result){
    auto hit=sylva::intersectPanel(*p,width,*ray);result[0]=hit.x;result[1]=hit.y;
    for(int i=0;i<3;i++)result[i+2]=hit.point.v[i];return hit.hit?1:0;
}
extern "C" void native_feedback_quad(const vr::HmdMatrix34_t* p,float width,const float* rect,vr::HmdMatrix34_t* pose,float* dimensions){auto q=sylva::feedbackQuad(*p,width,rect[0],rect[1],rect[2],rect[3]);*pose=q.pose;dimensions[0]=q.width;dimensions[1]=q.aspect;}
extern "C" int native_feedback_slice(const vr::HmdMatrix34_t* p,float width,const float* rect,unsigned part,vr::HmdMatrix34_t* pose,float* dimensions){
    std::array<sylva::FeedbackSlice,3> slices{};
    if(part>=3||!sylva::feedbackSlices(*p,width,rect[0],rect[1],rect[2],rect[3],slices))return 0;
    const auto& s=slices[part];*pose=s.quad.pose;dimensions[0]=s.quad.width;dimensions[1]=s.quad.aspect;
    dimensions[2]=(s.uv.uMax-s.uv.uMin)*sylva::feedbackAtlasWidth;dimensions[3]=(s.uv.vMax-s.uv.vMin)*sylva::feedbackAtlasHeight;return 1;
}
extern "C" unsigned native_feedback_alpha(float w,float h,float x,float y){static const auto atlas=sylva::feedbackPixels();return sylva::feedbackAlpha(atlas,w,h,x,y);}
// Explicit offline preview only. Use the production atlas, UVs and transforms,
// including OpenVR's cropped texture aspect; never create or touch VR overlays.
extern "C" int native_feedback_preview(const float* rect,int pressed){
    if(!offscreen||!cpuPanel||vrReady)return 0;
    vr::HmdMatrix34_t identity{};for(int i=0;i<3;i++)identity.m[i][i]=1;
    std::array<sylva::FeedbackSlice,3> slices{};
    if(!sylva::feedbackSlices(identity,W,rect[0],rect[1],rect[2],rect[3],slices))return 0;
    auto atlas=sylva::feedbackPixels();float red=pressed?.20f:.46f,green=pressed?.64f:.70f,blue=pressed?.40f:.55f,alpha=pressed?.85f:.48f;
    for(size_t i=0;i<atlas.size();i+=4){float a=atlas[i+3]*alpha;atlas[i]=(unsigned char)std::lround(blue*a);atlas[i+1]=(unsigned char)std::lround(green*a);atlas[i+2]=(unsigned char)std::lround(red*a);atlas[i+3]=(unsigned char)std::lround(a);}
    ComPtr<ID2D1Bitmap> bitmap;auto props=D2D1::BitmapProperties(D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM,D2D1_ALPHA_MODE_PREMULTIPLIED),144,144);
    if(!check(paint->CreateBitmap(D2D1::SizeU(sylva::feedbackAtlasWidth,sylva::feedbackAtlasHeight),atlas.data(),sylva::feedbackAtlasWidth*4,&props,&bitmap),"Feedback preview bitmap"))return 0;
    paint->BeginDraw();
    for(const auto& s:slices){
        float aspect=(s.uv.uMax-s.uv.uMin)*sylva::feedbackAtlasWidth/((s.uv.vMax-s.uv.vMin)*sylva::feedbackAtlasHeight)*s.quad.aspect;
        float height=s.quad.width/aspect,x=W*.5f+s.quad.pose.m[0][3]-s.quad.width*.5f,y=H*.5f-s.quad.pose.m[1][3]-height*.5f;
        auto target=D2D1::RectF(x,y,x+s.quad.width,y+height);
        auto source=D2D1::RectF(s.uv.uMin*sylva::feedbackAtlasWidth/1.5f,s.uv.vMin*sylva::feedbackAtlasHeight/1.5f,s.uv.uMax*sylva::feedbackAtlasWidth/1.5f,s.uv.vMax*sylva::feedbackAtlasHeight/1.5f);
        paint->DrawBitmap(bitmap.Get(),target,1,D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,source);
    }
    return check(paint->EndDraw(),"Feedback preview draw")&&check(software.prepare(0,0,TW,TH),"Feedback preview pixels")?1:0;
}
// Test/export path only: render our own panel texture to PNG, never capture a desktop/window.
// Normal interactive rendering never maps or reads back a GPU texture.
extern "C" int native_export(const unsigned short* path){
    if(cpuPanel)return check(software.exportPng(reinterpret_cast<const wchar_t*>(path)),"Export software PNG")?1:0;
    D3D11_TEXTURE2D_DESC desc{};texture->GetDesc(&desc);desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    ComPtr<ID3D11Texture2D> copy;if(!check(device->CreateTexture2D(&desc,nullptr,&copy),"Export texture"))return 0;
    context->CopyResource(copy.Get(),texture.Get());D3D11_MAPPED_SUBRESOURCE pixels{};
    if(!check(context->Map(copy.Get(),0,D3D11_MAP_READ,0,&pixels),"Export map"))return 0;
    ComPtr<IWICImagingFactory> wic;ComPtr<IWICStream> stream;ComPtr<IWICBitmapEncoder> encoder;ComPtr<IWICBitmapFrameEncode> frame;
    HRESULT hr=CoCreateInstance(CLSID_WICImagingFactory,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&wic));
    if(SUCCEEDED(hr))hr=wic->CreateStream(&stream);
    if(SUCCEEDED(hr))hr=stream->InitializeFromFilename(reinterpret_cast<const wchar_t*>(path),GENERIC_WRITE);
    if(SUCCEEDED(hr))hr=wic->CreateEncoder(GUID_ContainerFormatPng,nullptr,&encoder);
    if(SUCCEEDED(hr))hr=encoder->Initialize(stream.Get(),WICBitmapEncoderNoCache);
    if(SUCCEEDED(hr))hr=encoder->CreateNewFrame(&frame,nullptr);
    if(SUCCEEDED(hr))hr=frame->Initialize(nullptr);
    if(SUCCEEDED(hr))hr=frame->SetSize(TW,TH);
    WICPixelFormatGUID format=GUID_WICPixelFormat32bppBGRA;
    if(SUCCEEDED(hr))hr=frame->SetPixelFormat(&format);
    if(SUCCEEDED(hr))hr=frame->WritePixels(TH,pixels.RowPitch,pixels.RowPitch*TH,reinterpret_cast<BYTE*>(pixels.pData));
    if(SUCCEEDED(hr))hr=frame->Commit();if(SUCCEEDED(hr))hr=encoder->Commit();context->Unmap(copy.Get(),0);
    return check(hr,"Export PNG")?1:0;
}
extern "C" int native_export_raw(const unsigned short* path){return cpuPanel&&check(software.exportPng(reinterpret_cast<const wchar_t*>(path),true),"Export raw PNG")?1:0;}
extern "C" void native_close(){
    draftLine=DraftLineLayout{};
    vrStop.store(true);if(vrStopEvent)SetEvent(vrStopEvent);if(vrThread.joinable())vrThread.join();
    if(vrReady&&!hiddenProbe){vr::VRActiveActionSet_t set{};set.ulActionSet=actions;vr::VRInput()->UpdateActionState(&set,sizeof(set),1);}
    stopVr();lifecycle::close();
    feedbackPixels.clear();
    if(waitTimer)CloseHandle(waitTimer);waitTimer=nullptr;
    if(frameTimer)CloseHandle(frameTimer);frameTimer=nullptr;
    if(inputWake)CloseHandle(inputWake);inputWake=nullptr;if(vrStopEvent)CloseHandle(vrStopEvent);vrStopEvent=nullptr;
    if(window)DestroyWindow(window);window=nullptr;graphicsMetrics.queries={};layouts.clear();formats.clear();textWidths.clear();for(auto& geometry:iconGeometry)geometry.Reset();iconStroke.Reset();brush.Reset();paint.Reset();software.close();target.Reset();desktopTarget.Reset();draw.Reset();texture.Reset();swap.Reset();context.Reset();device.Reset();fonts.Reset();CoUninitialize();
}
extern "C" void native_message(const unsigned short* message){MessageBoxW(window,reinterpret_cast<const wchar_t*>(message),L"Sylva VR",MB_OK|MB_ICONERROR);}

extern "C" int native_forward_check(int steam){return lifecycle::acquire(steam!=0)==2;}
// Regression check runs only its own window procedure. No engine, OSC, HMD,
// user settings or other application's UI is accessed.
extern "C" int native_lifecycle_check(){
    int checks=0;
    auto require=[&](bool ok,const char* name){if(!ok)snprintf(lastError,sizeof(lastError),"Lifecycle check: %s",name);else checks++;return ok;};
    auto hasEvent=[](int kind){std::lock_guard<std::mutex> lock(eventMutex);bool found=false;for(auto e:events)found|=e.kind==kind;events.clear();return found;};
    auto forward=[&](bool steam){
        wchar_t exe[32768]{};GetModuleFileNameW(nullptr,exe,32768);
        std::wstring cmd=L"\""+std::wstring(exe)+L"\" --lifecycle-forward-check"+(steam?L" steam":L"");
        STARTUPINFOW si{};si.cb=sizeof(si);si.dwFlags=STARTF_USESHOWWINDOW;si.wShowWindow=SW_HIDE;PROCESS_INFORMATION pi{};
        if(!CreateProcessW(exe,cmd.data(),nullptr,nullptr,FALSE,CREATE_NO_WINDOW,nullptr,nullptr,&si,&pi))return false;
        DWORD wait=WaitForSingleObject(pi.hProcess,10000),code=1;
        if(wait==WAIT_TIMEOUT){TerminateProcess(pi.hProcess,1);WaitForSingleObject(pi.hProcess,2000);}
        GetExitCodeProcess(pi.hProcess,&code);CloseHandle(pi.hThread);CloseHandle(pi.hProcess);
        native_pump();return wait==WAIT_OBJECT_0&&code==0;
    };
    if(native_init("",6,1|2|4|8)!=1)return 0;
    auto run=[&](){
        if(!require(lifecycle::trayAdded,"notification icon added"))return false;
        if(!require(!IsWindowVisible(window)&&!visible,"silent desktop and VR state"))return false;
        if(!require(lifecycle::appIcon(16)&&lifecycle::appIcon(32)&&lifecycle::appIcon(256),"embedded icon sizes"))return false;
        HMENU menu=lifecycle::menu();wchar_t first[64]{},second[64]{};
        GetMenuStringW(menu,0,first,64,MF_BYPOSITION);GetMenuStringW(menu,1,second,64,MF_BYPOSITION);
        bool exact=GetMenuItemCount(menu)==2&&std::wstring(first)==L"显示主窗口"&&std::wstring(second)==L"退出";DestroyMenu(menu);
        if(!require(exact,"two menu labels"))return false;
        hasEvent(1);
        if(!require(forward(true)&&hasEvent(10)&&!IsWindowVisible(window),"SteamVR launch forwarded without showing"))return false;
        if(!require(forward(false)&&IsWindowVisible(window),"second launch restores first instance"))return false;
        hasEvent(1);SendMessageW(window,WM_CLOSE,0,0);
        if(!require(!IsWindowVisible(window)&&!hasEvent(1),"close hides without exiting"))return false;
        SendMessageW(window,WM_COMMAND,lifecycle::showCommand,0);
        if(!require(IsWindowVisible(window),"tray show restores window"))return false;
        SendMessageW(window,WM_CLOSE,0,0);
        Shell_NotifyIconW(NIM_DELETE,&lifecycle::icon);
        SendMessageW(window,lifecycle::taskbarCreated,0,0);
        if(!require(lifecycle::trayAdded&&!IsWindowVisible(window),"Explorer restart recovery"))return false;
        hasEvent(1);SendMessageW(window,WM_COMMAND,lifecycle::exitCommand,0);
        if(!require(hasEvent(1),"tray exit overrides close-to-tray"))return false;
        native_lifecycle_settings(0);SendMessageW(window,WM_CLOSE,0,0);
        if(!require(hasEvent(1),"close exits when preference disabled"))return false;
        return true;
    };
    bool ok=run();native_close();
    if(!ok)return 0;
    if(!require(!lifecycle::trayAdded&&!lifecycle::instanceMutex&&!window,"shutdown releases shell and mutex resources"))return 0;
    return checks;
}
