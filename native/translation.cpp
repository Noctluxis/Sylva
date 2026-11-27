// SPDX-License-Identifier: MPL-2.0
#define NOMINMAX
#include <windows.h>
#include <winhttp.h>
#include <wincred.h>
#include <bcrypt.h>
#include <string>
#include <vector>
#include <cstring>
#include <cstdlib>

struct InternetHandle {
    HINTERNET value;
    ~InternetHandle() { if (value) WinHttpCloseHandle(value); }
};
static HINTERNET connectionPool() {
    static InternetHandle session{WinHttpOpen(L"Sylva/1.0", WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0)};
    return session.value;
}
extern "C" DWORD translation_http(const wchar_t* method, const wchar_t* url, const wchar_t* headers,
    const char* body, DWORD bodyLen, char** output, DWORD* length, DWORD* status) {
    URL_COMPONENTS u{}; u.dwStructSize=sizeof(u);
    u.dwHostNameLength=u.dwUrlPathLength=u.dwExtraInfoLength=u.dwUserNameLength=u.dwPasswordLength=DWORD(-1);
    if (!WinHttpCrackUrl(url,0,0,&u)) return GetLastError();
    std::wstring host(u.lpszHostName,u.dwHostNameLength);
    const bool secure=u.nScheme==INTERNET_SCHEME_HTTPS;
    if ((!secure && (u.nScheme!=INTERNET_SCHEME_HTTP || (host!=L"127.0.0.1" && host!=L"localhost" && host!=L"[::1]"))) ||
        u.dwUserNameLength || u.dwPasswordLength) return ERROR_INVALID_PARAMETER;
    std::wstring path=u.dwUrlPathLength?std::wstring(u.lpszUrlPath,u.dwUrlPathLength):L"/";
    if (u.dwExtraInfoLength) path.append(u.lpszExtraInfo,u.dwExtraInfoLength);
    HINTERNET session=connectionPool();
    if (!session) return ERROR_WINHTTP_INTERNAL_ERROR;
    InternetHandle server{WinHttpConnect(session,host.c_str(),u.nPort,0)};
    if (!server.value) return GetLastError();
    InternetHandle req{WinHttpOpenRequest(server.value,method,path.c_str(),nullptr,nullptr,
        WINHTTP_DEFAULT_ACCEPT_TYPES,secure?WINHTTP_FLAG_SECURE:0)};
    if (!req.value) return GetLastError();
    DWORD disable=WINHTTP_DISABLE_REDIRECTS;
    if (!WinHttpSetOption(req.value,WINHTTP_OPTION_DISABLE_FEATURE,&disable,sizeof(disable))) return GetLastError();
    WinHttpSetTimeouts(req.value,5000,5000,8000,12000);
    DWORD decompression=WINHTTP_DECOMPRESSION_FLAG_GZIP|WINHTTP_DECOMPRESSION_FLAG_DEFLATE;
    WinHttpSetOption(req.value,WINHTTP_OPTION_DECOMPRESSION,&decompression,sizeof(decompression));
    if (!WinHttpSendRequest(req.value,headers,DWORD(-1),const_cast<char*>(body),bodyLen,bodyLen,0) ||
        !WinHttpReceiveResponse(req.value,nullptr)) return GetLastError();
    DWORD size=sizeof(*status);
    if (!WinHttpQueryHeaders(req.value,WINHTTP_QUERY_STATUS_CODE|WINHTTP_QUERY_FLAG_NUMBER,nullptr,status,&size,nullptr)) return GetLastError();
    if (*status<200 || *status>=300) return 0;
    std::string bytes;
    char block[8192];
    for (;;) {
        DWORD n=0;
        if (!WinHttpReadData(req.value,block,sizeof(block),&n)) return GetLastError();
        if (!n) break;
        if (bytes.size()+n>2*1024*1024) return ERROR_FILE_TOO_LARGE;
        bytes.append(block,n);
    }
    if (!bytes.empty()) {
        *output=static_cast<char*>(std::malloc(bytes.size()));
        if (!*output) return ERROR_OUTOFMEMORY;
        std::memcpy(*output,bytes.data(),bytes.size());
    }
    *length=DWORD(bytes.size());
    return 0;
}
extern "C" void translation_free(char* data) { std::free(data); }

extern "C" DWORD translation_credential(const wchar_t* target, BYTE* bytes, DWORD* length, BOOL write) {
    if (write) {
        CREDENTIALW c{};
        c.Type=CRED_TYPE_GENERIC; c.TargetName=const_cast<wchar_t*>(target);
        c.Persist=CRED_PERSIST_LOCAL_MACHINE; c.CredentialBlob=bytes; c.CredentialBlobSize=*length;
        return CredWriteW(&c,0)?0:GetLastError();
    }
    PCREDENTIALW c=nullptr;
    if (!CredReadW(target,CRED_TYPE_GENERIC,0,&c)) return GetLastError();
    DWORD error=0;
    if (*length<c->CredentialBlobSize) error=ERROR_INSUFFICIENT_BUFFER;
    else { *length=c->CredentialBlobSize; if (*length) std::memcpy(bytes,c->CredentialBlob,*length); }
    CredFree(c);
    return error;
}
extern "C" int translation_digest(const wchar_t* algorithm,const BYTE* key,DWORD keyLen,const BYTE* data,DWORD size,BYTE* out,DWORD len) {
    BCRYPT_ALG_HANDLE handle=nullptr;
    if(BCryptOpenAlgorithmProvider(&handle,algorithm,nullptr,keyLen?BCRYPT_ALG_HANDLE_HMAC_FLAG:0)<0) return 0;
    auto result=BCryptHash(handle,const_cast<BYTE*>(key),keyLen,const_cast<BYTE*>(data),size,out,len);
    BCryptCloseAlgorithmProvider(handle,0);
    return result>=0;
}
