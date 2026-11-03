cmake_minimum_required(VERSION 3.25)

project(badi-fcitx5 VERSION 0.1.0 LANGUAGES CXX)

include(GNUInstallDirs)
find_package(Fcitx5Core 5.1.7 REQUIRED)
find_package(nlohmann_json 3.11 REQUIRED)

# Warnings fail development and CI builds; a distribution build turns that off
# (-DBADI_WERROR=OFF) so a newer compiler's new warning cannot break it.
option(BADI_WERROR "Treat compiler warnings as errors" ON)
set(BADI_WARNINGS -Wall -Wextra -Wpedantic)
if(BADI_WERROR)
    list(APPEND BADI_WARNINGS -Werror)
endif()

add_library(badi-fcitx5-core STATIC
    src/accessibility.cpp
    src/debug.cpp
    src/observation.cpp
    src/revision.cpp
    src/sanitizer.cpp
    src/state.cpp
    src/transport.cpp
)
set_target_properties(badi-fcitx5-core PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_compile_features(badi-fcitx5-core PUBLIC cxx_std_20)
target_include_directories(badi-fcitx5-core PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/src")
target_link_libraries(badi-fcitx5-core PUBLIC Fcitx5::Core nlohmann_json::nlohmann_json)
target_compile_options(badi-fcitx5-core PRIVATE ${BADI_WARNINGS})

set(BADI_ADDON_SOURCES
    src/addon.cpp
    src/addon_broker.cpp
    src/addon_keys.cpp
    src/addon_observer.cpp
    src/addon_revision.cpp
    src/addon_session.cpp
)
if(COMMAND add_fcitx5_addon)
    add_fcitx5_addon(badi-fcitx5 ${BADI_ADDON_SOURCES})
else()
    # Fcitx 5.1.7 (Ubuntu 24.04) predates add_fcitx5_addon. Build what the
    # macro builds by default: a loadable MODULE marked as a Fcitx addon,
    # libbadi-fcitx5.so, which badi.conf names as a SharedLibrary.
    add_library(badi-fcitx5 MODULE ${BADI_ADDON_SOURCES})
    set_target_properties(badi-fcitx5 PROPERTIES FCITX_ADDON TRUE)
endif()
target_compile_features(badi-fcitx5 PRIVATE cxx_std_20)
target_link_libraries(badi-fcitx5 PRIVATE badi-fcitx5-core Fcitx5::Core)
target_compile_options(badi-fcitx5 PRIVATE ${BADI_WARNINGS})
set_target_properties(badi-fcitx5 PROPERTIES OUTPUT_NAME badi-fcitx5)

configure_file(badi.conf.in badi.conf @ONLY)
install(TARGETS badi-fcitx5 DESTINATION "${CMAKE_INSTALL_LIBDIR}/fcitx5")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/badi.conf"
        DESTINATION "${CMAKE_INSTALL_DATADIR}/fcitx5/addon")

include(CTest)
if(BUILD_TESTING)
    add_executable(badi-fcitx5-replacement-tests tests/replacement.cpp)
    target_compile_features(badi-fcitx5-replacement-tests PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-replacement-tests PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-replacement-tests PRIVATE ${BADI_WARNINGS})
    add_test(NAME badi-fcitx5-replacement-tests COMMAND badi-fcitx5-replacement-tests)
    add_executable(badi-fcitx5-accessibility-tests tests/accessibility.cpp)
    target_compile_features(badi-fcitx5-accessibility-tests PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-accessibility-tests PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-accessibility-tests PRIVATE ${BADI_WARNINGS})
    add_test(NAME badi-fcitx5-accessibility-tests COMMAND badi-fcitx5-accessibility-tests)
    add_executable(badi-fcitx5-transport-tests tests/transport.cpp)
    target_compile_features(badi-fcitx5-transport-tests PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-transport-tests PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-transport-tests PRIVATE ${BADI_WARNINGS})
    add_test(NAME badi-fcitx5-transport-tests COMMAND badi-fcitx5-transport-tests)
    add_executable(badi-fcitx5-policy-tests tests/policy.cpp)
    target_compile_features(badi-fcitx5-policy-tests PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-policy-tests PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-policy-tests PRIVATE ${BADI_WARNINGS})
    add_test(NAME badi-fcitx5-policy-tests COMMAND badi-fcitx5-policy-tests)
    add_executable(badi-fcitx5-debug-tests tests/debug.cpp)
    target_compile_features(badi-fcitx5-debug-tests PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-debug-tests PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-debug-tests PRIVATE ${BADI_WARNINGS})
    add_test(NAME badi-fcitx5-debug-tests COMMAND badi-fcitx5-debug-tests)
    add_executable(badi-fcitx5-revision-tests tests/revision.cpp)
    target_compile_features(badi-fcitx5-revision-tests PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-revision-tests PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-revision-tests PRIVATE ${BADI_WARNINGS})
    add_test(NAME badi-fcitx5-revision-tests COMMAND badi-fcitx5-revision-tests)
    add_executable(badi-fcitx5-tests tests/tests.cpp)
    target_compile_features(badi-fcitx5-tests PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-tests PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-tests PRIVATE ${BADI_WARNINGS})
    add_test(NAME badi-fcitx5-tests COMMAND badi-fcitx5-tests
        "${CMAKE_CURRENT_SOURCE_DIR}/../../protocol/orthographic-joiner-fixtures.json")

    find_package(Python3 REQUIRED COMPONENTS Interpreter)
    add_test(NAME badi-fcitx5-observer-identities
        COMMAND "${Python3_EXECUTABLE}" "${CMAKE_CURRENT_SOURCE_DIR}/tests/observer-identities.py")

    add_executable(badi-fcitx5-broker-smoke tests/broker-smoke.cpp)
    target_compile_features(badi-fcitx5-broker-smoke PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-broker-smoke PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-broker-smoke PRIVATE ${BADI_WARNINGS})
endif()
