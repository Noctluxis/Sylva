// SPDX-License-Identifier: MPL-2.0
#pragma once
#include <algorithm>
#include <cmath>
namespace sylva {
// Remaining deadline budget includes BOTH work and any runtime wait. Rounding
// elapsed time to integer milliseconds can accidentally add a second sleep.
inline long long remainingTicks(double hz,double elapsedMs) {
    if(!std::isfinite(hz)||hz<=0||!std::isfinite(elapsedMs))return 0;
    return static_cast<long long>(std::ceil(std::max(0.,1000./hz-elapsedMs)*10000.));
}
}
