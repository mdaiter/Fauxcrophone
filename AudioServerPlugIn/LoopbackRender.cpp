#include "LoopbackBridge.h"
#include <AudioServerPlugIn.h>
#include <CoreAudio/CoreAudio.h>
#include <os/log.h>

static LoopbackMixerHandle gMixerHandle = nullptr;

extern "C" OSStatus LoopbackInitialize(double sampleRate, uint32_t maxFrames) {
    if (gMixerHandle != nullptr) {
        return noErr;
    }
    gMixerHandle = loopback_mixer_create(sampleRate, maxFrames);
    return gMixerHandle != nullptr ? noErr : kAudioHardwareUnspecifiedError;
}

extern "C" void LoopbackShutdown() {
    if (gMixerHandle != nullptr) {
        loopback_mixer_destroy(gMixerHandle);
        gMixerHandle = nullptr;
    }
}

extern "C" OSStatus LoopbackProcess(AudioServerPlugInIOOperationData* ioData) {
    if (gMixerHandle == nullptr) {
        return kAudioHardwareUnspecifiedError;
    }

    auto output = static_cast<AudioBufferList*>(ioData->ioBufferList);
    auto timestamp = static_cast<AudioTimeStamp*>(ioData->inOutputTime);
    if (output == nullptr || timestamp == nullptr) {
        return kAudioHardwareUnspecifiedError;
    }

    const uint32_t frameCount = ioData->inNumberFrames;
    LoopbackRenderArgs args{
        .bufferList = output,
        .frameCount = frameCount,
        .timestamp = timestamp
    };

    OSStatus status = loopback_mixer_process(gMixerHandle, &args);
    const char* logLine = nullptr;
    while ((logLine = device_kit_pop_log())) {
        os_log(OS_LOG_DEFAULT, "[Rust] %s", logLine);
    }
    return status;
}
