#pragma once
#include <string_view>

namespace std {
// #if __cplusplus >= 201703L
// Only define if libc++ doesn't already have it
inline const char* c_str(std::string_view sv) noexcept {
    return sv.data();
}
// #endif
}
