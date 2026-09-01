cmake_minimum_required(VERSION 3.25)

project(badi-fcitx5 VERSION 0.1.0 LANGUAGES CXX)

include(GNUInstallDirs)
find_package(Fcitx5Core 5.1 REQUIRED)
find_package(nlohmann_json 3.11 REQUIRED)

add_library(badi-fcitx5-core STATIC
    src/sanitizer.cpp
    src/state.cpp
    src/transport.cpp
)
set_target_properties(badi-fcitx5-core PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_compile_features(badi-fcitx5-core PUBLIC cxx_std_20)
target_include_directories(badi-fcitx5-core PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/src")
target_link_libraries(badi-fcitx5-core PUBLIC Fcitx5::Core nlohmann_json::nlohmann_json)
target_compile_options(badi-fcitx5-core PRIVATE -Wall -Wextra -Wpedantic -Werror)

add_fcitx5_addon(badi-fcitx5 src/addon.cpp)
target_compile_features(badi-fcitx5 PRIVATE cxx_std_20)
target_link_libraries(badi-fcitx5 PRIVATE badi-fcitx5-core Fcitx5::Core)
target_compile_options(badi-fcitx5 PRIVATE -Wall -Wextra -Wpedantic -Werror)
set_target_properties(badi-fcitx5 PROPERTIES OUTPUT_NAME badi-fcitx5)

configure_file(badi.conf.in badi.conf @ONLY)
install(TARGETS badi-fcitx5 DESTINATION "${CMAKE_INSTALL_LIBDIR}/fcitx5")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/badi.conf"
        DESTINATION "${CMAKE_INSTALL_DATADIR}/fcitx5/addon")

include(CTest)
if(BUILD_TESTING)
    add_executable(badi-fcitx5-tests tests/tests.cpp)
    target_compile_features(badi-fcitx5-tests PRIVATE cxx_std_20)
    target_link_libraries(badi-fcitx5-tests PRIVATE badi-fcitx5-core)
    target_compile_options(badi-fcitx5-tests PRIVATE -Wall -Wextra -Wpedantic -Werror)
    add_test(NAME badi-fcitx5-tests COMMAND badi-fcitx5-tests)
endif()
