// SPDX-License-Identifier: MPL-2.0
#pragma once
#include <shellapi.h>

// Native shell icon: no polling thread, runtime library or hidden helper app.
namespace lifecycle {
constexpr UINT trayMessage=WM_APP+51, activateMessage=WM_APP+52;
constexpr UINT showCommand=1, exitCommand=2;
static HANDLE instanceMutex=nullptr;
static bool closeToTray=true, trayAdded=false;
static UINT taskbarCreated=0;
static NOTIFYICONDATAW icon{};
static HICON appIcon(int size){
    return (HICON)LoadImageW(GetModuleHandleW(nullptr),MAKEINTRESOURCEW(101),IMAGE_ICON,size,size,LR_SHARED);
}
// Return 2 after handing a second launch to the first process; 0 on failure.
static int acquire(bool fromSteamVr){
    instanceMutex=CreateMutexW(nullptr,FALSE,L"Local\\SylvaVR.Application");
    if(!instanceMutex)return 0;
    if(GetLastError()!=ERROR_ALREADY_EXISTS)return 1;
    CloseHandle(instanceMutex);instanceMutex=nullptr;
    for(int i=0;i<100;i++){
        HWND existing=FindWindowW(L"SylvaVRPanel",nullptr);
        if(existing){
            DWORD pid=0;GetWindowThreadProcessId(existing,&pid);AllowSetForegroundWindow(pid);
            if(PostMessageW(existing,activateMessage,fromSteamVr?1:0,0))return 2;
        }
        Sleep(50);
    }
    return 0;
}
static bool addTray(HWND hwnd){
    icon={};icon.cbSize=sizeof(icon);icon.hWnd=hwnd;icon.uID=1;
    icon.uFlags=NIF_MESSAGE|NIF_ICON|NIF_TIP|NIF_SHOWTIP;
    icon.uCallbackMessage=trayMessage;icon.hIcon=appIcon(GetSystemMetrics(SM_CXSMICON));
    wcscpy_s(icon.szTip,L"Sylva · VR 中文输入法");
    trayAdded=Shell_NotifyIconW(NIM_ADD,&icon)!=FALSE;
    if(trayAdded){icon.uVersion=NOTIFYICON_VERSION_4;Shell_NotifyIconW(NIM_SETVERSION,&icon);}
    taskbarCreated=RegisterWindowMessageW(L"TaskbarCreated");
    return trayAdded;
}
static void show(HWND hwnd){
    // A SteamVR/shortcut launcher may supply STARTF_USESHOWWINDOW/SW_HIDE.
    // Windows consumes that hint on the first ShowWindow call, even when we
    // explicitly request a restore. The second call honors the user's Show.
    ShowWindow(hwnd,SW_RESTORE);
    ShowWindow(hwnd,SW_SHOW);
    SetForegroundWindow(hwnd);InvalidateRect(hwnd,nullptr,FALSE);
}
static HMENU menu(){
    HMENU m=CreatePopupMenu();
    AppendMenuW(m,MF_STRING,showCommand,L"显示主窗口");
    AppendMenuW(m,MF_STRING,exitCommand,L"退出");
    return m;
}
static UINT popup(HWND hwnd,LPARAM lp,WPARAM wp){
    if(LOWORD(lp)==NIN_SELECT||LOWORD(lp)==NIN_KEYSELECT)return showCommand;
    if(LOWORD(lp)!=WM_CONTEXTMENU)return 0;
    POINT point{GET_X_LPARAM(wp),GET_Y_LPARAM(wp)};
    if(point.x==-1&&point.y==-1)GetCursorPos(&point);
    HMENU m=menu();SetForegroundWindow(hwnd);
    UINT command=TrackPopupMenuEx(m,TPM_RETURNCMD|TPM_NONOTIFY|TPM_RIGHTBUTTON,point.x,point.y,hwnd,nullptr);
    DestroyMenu(m);PostMessageW(hwnd,WM_NULL,0,0);return command;
}
static void close(){
    if(trayAdded){Shell_NotifyIconW(NIM_DELETE,&icon);trayAdded=false;}
    if(instanceMutex){CloseHandle(instanceMutex);instanceMutex=nullptr;}
}
}
