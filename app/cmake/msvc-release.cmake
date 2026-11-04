# For C libraries built through the cmake crate on Windows (libopus, for opusic-sys): under the
# Visual Studio generator that crate writes CMAKE_<LANG>_FLAGS_RELEASE itself, without the
# optimisation CMake's own Release has, so MSVC compiled them unoptimised. This puts it back.
foreach(lang C CXX)
  if(NOT CMAKE_${lang}_FLAGS_RELEASE MATCHES "/O2")
    set(CMAKE_${lang}_FLAGS_RELEASE "${CMAKE_${lang}_FLAGS_RELEASE} /O2 /Ob2 /DNDEBUG" CACHE STRING "" FORCE)
  endif()
endforeach()
