# Install script for directory: /home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls

# Set the install prefix
if(NOT DEFINED CMAKE_INSTALL_PREFIX)
  set(CMAKE_INSTALL_PREFIX "/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-install")
endif()
string(REGEX REPLACE "/$" "" CMAKE_INSTALL_PREFIX "${CMAKE_INSTALL_PREFIX}")

# Set the install configuration name.
if(NOT DEFINED CMAKE_INSTALL_CONFIG_NAME)
  if(BUILD_TYPE)
    string(REGEX REPLACE "^[^A-Za-z0-9_]+" ""
           CMAKE_INSTALL_CONFIG_NAME "${BUILD_TYPE}")
  else()
    set(CMAKE_INSTALL_CONFIG_NAME "MinSizeRel")
  endif()
  message(STATUS "Install configuration: \"${CMAKE_INSTALL_CONFIG_NAME}\"")
endif()

# Set the component getting installed.
if(NOT CMAKE_INSTALL_COMPONENT)
  if(COMPONENT)
    message(STATUS "Install component: \"${COMPONENT}\"")
    set(CMAKE_INSTALL_COMPONENT "${COMPONENT}")
  else()
    set(CMAKE_INSTALL_COMPONENT)
  endif()
endif()

# Is this installation the result of a crosscompile?
if(NOT DEFINED CMAKE_CROSSCOMPILING)
  set(CMAKE_CROSSCOMPILING "TRUE")
endif()

# Set default install directory permissions.
if(NOT DEFINED CMAKE_OBJDUMP)
  set(CMAKE_OBJDUMP "/opt/xpack/xpack-arm-none-eabi-gcc-14.2.1-1.1/bin/arm-none-eabi-objdump")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/MbedTLS" TYPE FILE FILES
    "/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/cmake/MbedTLSConfig.cmake"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/cmake/MbedTLSConfigVersion.cmake"
    )
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  if(EXISTS "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/MbedTLS/MbedTLSTargets.cmake")
    file(DIFFERENT _cmake_export_file_changed FILES
         "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/MbedTLS/MbedTLSTargets.cmake"
         "/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/CMakeFiles/Export/501c3fe65339b3965ef28cb8bf064996/MbedTLSTargets.cmake")
    if(_cmake_export_file_changed)
      file(GLOB _cmake_old_config_files "$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/MbedTLS/MbedTLSTargets-*.cmake")
      if(_cmake_old_config_files)
        string(REPLACE ";" ", " _cmake_old_config_files_text "${_cmake_old_config_files}")
        message(STATUS "Old export file \"$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/cmake/MbedTLS/MbedTLSTargets.cmake\" will be replaced.  Removing files [${_cmake_old_config_files_text}].")
        unset(_cmake_old_config_files_text)
        file(REMOVE ${_cmake_old_config_files})
      endif()
      unset(_cmake_old_config_files)
    endif()
    unset(_cmake_export_file_changed)
  endif()
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/MbedTLS" TYPE FILE FILES "/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/CMakeFiles/Export/501c3fe65339b3965ef28cb8bf064996/MbedTLSTargets.cmake")
  if(CMAKE_INSTALL_CONFIG_NAME MATCHES "^([Mm][Ii][Nn][Ss][Ii][Zz][Ee][Rr][Ee][Ll])$")
    file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib/cmake/MbedTLS" TYPE FILE FILES "/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/CMakeFiles/Export/501c3fe65339b3965ef28cb8bf064996/MbedTLSTargets-minsizerel.cmake")
  endif()
endif()

if(NOT CMAKE_INSTALL_LOCAL_ONLY)
  # Include the install script for each subdirectory.
  include("/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/framework/cmake_install.cmake")
  include("/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/include/cmake_install.cmake")
  include("/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/3rdparty/cmake_install.cmake")
  include("/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/library/cmake_install.cmake")
  include("/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/pkgconfig/cmake_install.cmake")

endif()

if(CMAKE_INSTALL_COMPONENT)
  set(CMAKE_INSTALL_MANIFEST "install_manifest_${CMAKE_INSTALL_COMPONENT}.txt")
else()
  set(CMAKE_INSTALL_MANIFEST "install_manifest.txt")
endif()

string(REPLACE ";" "\n" CMAKE_INSTALL_MANIFEST_CONTENT
       "${CMAKE_INSTALL_MANIFEST_FILES}")
file(WRITE "/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s3-srt/mbedtls-build/${CMAKE_INSTALL_MANIFEST}"
     "${CMAKE_INSTALL_MANIFEST_CONTENT}")
