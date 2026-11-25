// SPDX-License-Identifier: MPL-2.0
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <xaudio2.h>

// Owned by the audio thread. Each short tap can finish while the other hand
// starts a second tap. Submitted buffers point into immutable executable data.
static IXAudio2* engine=nullptr;
static IXAudio2MasteringVoice* output=nullptr;
static IXAudio2SourceVoice* voices[4]{};
static bool initializedCom=false;

extern "C" void key_audio_close(){
    for(auto& voice:voices){
        if(voice)voice->DestroyVoice();
        voice=nullptr;
    }
    if(output)output->DestroyVoice();
    output=nullptr;
    if(engine)engine->Release();
    engine=nullptr;
    if(initializedCom)CoUninitialize();
    initializedCom=false;
}

static bool openDevice(){
    initializedCom=SUCCEEDED(CoInitializeEx(nullptr,COINIT_MULTITHREADED));
    if(!initializedCom || FAILED(XAudio2Create(&engine)) ||
       FAILED(engine->CreateMasteringVoice(&output)))return false;
    WAVEFORMATEX format{WAVE_FORMAT_PCM,1,24000,48000,2,16,0};
    for(auto& voice:voices){
        if(FAILED(engine->CreateSourceVoice(&voice,&format,XAUDIO2_VOICE_NOPITCH)) ||
           FAILED(voice->Start()))return false;
    }
    return true;
}

extern "C" bool key_audio_play(const unsigned char* pcm, unsigned bytes){
    if(!engine&&!openDevice()){key_audio_close();return false;}
    XAUDIO2_BUFFER buffer{};
    buffer.AudioBytes=bytes;
    buffer.pAudioData=pcm;
    buffer.Flags=XAUDIO2_END_OF_STREAM;
    for(auto voice:voices){
        XAUDIO2_VOICE_STATE state{};
        voice->GetState(&state,XAUDIO2_VOICE_NOSAMPLESPLAYED);
        if(state.BuffersQueued==0){
            if(SUCCEEDED(voice->SubmitSourceBuffer(&buffer)))return true;
            key_audio_close();
            return false;
        }
    }
    return true; // Prefer dropping a burst to replaying a stale tap.
}
