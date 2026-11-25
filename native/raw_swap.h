// SPDX-License-Identifier: MPL-2.0
#pragma once
#include <mutex>

namespace sylva {
// UI uploads into the hidden slot. The VR thread may release the previous
// front only AFTER presenting the ready slot. Holding the reservation through
// handoff prevents the UI from overwriting either displayed texture.
class RawSwap {
    mutable std::mutex mutex;
    int frontSlot=-1,loadingSlot=-1,readySlot=-1;
public:
    int begin(){std::lock_guard<std::mutex> lock(mutex);if(loadingSlot>=0)return -1;loadingSlot=frontSlot==0?1:0;return loadingSlot;}
    bool loaded(int slot){std::lock_guard<std::mutex> lock(mutex);if(slot<0||slot!=loadingSlot||readySlot>=0)return false;readySlot=slot;return true;}
    int front()const{std::lock_guard<std::mutex> lock(mutex);return frontSlot;}
    int loading()const{std::lock_guard<std::mutex> lock(mutex);return loadingSlot;}
    int ready()const{std::lock_guard<std::mutex> lock(mutex);return readySlot;}
    bool presented(int slot){std::lock_guard<std::mutex> lock(mutex);if(slot<0||slot!=readySlot)return false;frontSlot=slot;loadingSlot=readySlot=-1;return true;}
    void failed(int slot){std::lock_guard<std::mutex> lock(mutex);if(slot==loadingSlot)loadingSlot=readySlot=-1;}
};
}
