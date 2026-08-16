cmake_minimum_required(VERSION 3.20)

file(READ "${CMAKE_CURRENT_SOURCE_DIR}/../Cargo.toml" BEAN_KEY_CARGO_TOML)
string(REGEX MATCH "version = \"([0-9]+\\.[0-9]+\\.[0-9]+)\"" _ "${BEAN_KEY_CARGO_TOML}")
if(NOT CMAKE_MATCH_1)
  message(FATAL_ERROR "Could not read the workspace version from Cargo.toml")
endif()

project(bean-key-fcitx5 VERSION "${CMAKE_MATCH_1}" LANGUAGES CXX)

include(GNUInstallDirs)
include(CTest)

find_package(Fcitx5Core 5.1.21 REQUIRED)
set(protobuf_MODULE_COMPATIBLE TRUE)
find_package(Protobuf CONFIG REQUIRED)
find_package(Threads REQUIRED)

set(BEAN_KEY_DAEMON_PATH "bean-key-daemon" CACHE FILEPATH "Absolute beanKey daemon path")
set(BEAN_KEY_CONFIG_PATH "/etc/bean-key/config.toml" CACHE FILEPATH "Internal daemon configuration path")
set(BEAN_KEY_REQUEST_TIMEOUT_MS "5000" CACHE STRING "Synchronous daemon request deadline")

protobuf_generate_cpp(
  BEAN_KEY_PROTO_SOURCES
  BEAN_KEY_PROTO_HEADERS
  ${CMAKE_CURRENT_SOURCE_DIR}/../proto/bean_key.proto
)

add_library(bean-key-ipc STATIC
  src/client.cpp
  ${BEAN_KEY_PROTO_SOURCES}
)
set_target_properties(bean-key-ipc PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_compile_features(bean-key-ipc PUBLIC cxx_std_17)
target_compile_options(bean-key-ipc PRIVATE -Wall -Wextra -Wpedantic -Werror)
target_include_directories(bean-key-ipc PUBLIC src ${CMAKE_CURRENT_BINARY_DIR})
target_link_libraries(bean-key-ipc PUBLIC protobuf::libprotobuf)

add_library(bean-key-key-event STATIC
  src/key_event.cpp
)
set_target_properties(bean-key-key-event PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_compile_features(bean-key-key-event PUBLIC cxx_std_20)
target_compile_options(bean-key-key-event PRIVATE -Wall -Wextra -Wpedantic -Werror)
target_link_libraries(bean-key-key-event PUBLIC bean-key-ipc Fcitx5::Core)

add_library(bean-key-engine STATIC
  src/engine.cpp
)
set_target_properties(bean-key-engine PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_compile_features(bean-key-engine PUBLIC cxx_std_20)
target_compile_options(bean-key-engine PRIVATE -Wall -Wextra -Wpedantic -Werror)
target_compile_definitions(bean-key-engine PRIVATE
  BEAN_KEY_DAEMON_PATH="${BEAN_KEY_DAEMON_PATH}"
  BEAN_KEY_CONFIG_PATH="${BEAN_KEY_CONFIG_PATH}"
  BEAN_KEY_REQUEST_TIMEOUT_MS=${BEAN_KEY_REQUEST_TIMEOUT_MS}
)
target_link_libraries(bean-key-engine PUBLIC bean-key-key-event Fcitx5::Core)

add_library(fcitx5-bean-key MODULE
  src/factory.cpp
)
set_target_properties(fcitx5-bean-key PROPERTIES PREFIX "")
target_compile_features(fcitx5-bean-key PRIVATE cxx_std_20)
target_compile_options(fcitx5-bean-key PRIVATE -Wall -Wextra -Wpedantic -Werror)
target_link_libraries(fcitx5-bean-key PRIVATE bean-key-engine)

install(TARGETS fcitx5-bean-key DESTINATION ${CMAKE_INSTALL_LIBDIR}/fcitx5)
install(FILES bean_key-addon.conf DESTINATION ${CMAKE_INSTALL_DATADIR}/fcitx5/addon RENAME bean_key.conf)
install(FILES bean_key-inputmethod.conf DESTINATION ${CMAKE_INSTALL_DATADIR}/fcitx5/inputmethod RENAME bean_key.conf)
install(FILES themes/beanKey/theme.conf DESTINATION ${CMAKE_INSTALL_DATADIR}/fcitx5/themes/beanKey)

if(BUILD_TESTING)
  add_executable(bean-key-client-test tests/client_test.cpp)
  target_compile_features(bean-key-client-test PRIVATE cxx_std_17)
  target_compile_options(bean-key-client-test PRIVATE -Wall -Wextra -Wpedantic -Werror)
  target_link_libraries(bean-key-client-test PRIVATE bean-key-ipc Threads::Threads)
  add_test(NAME bean-key-client COMMAND bean-key-client-test)

  add_executable(bean-key-key-event-test tests/key_event_test.cpp)
  target_compile_features(bean-key-key-event-test PRIVATE cxx_std_20)
  target_compile_options(bean-key-key-event-test PRIVATE -Wall -Wextra -Wpedantic -Werror)
  target_link_libraries(bean-key-key-event-test PRIVATE bean-key-key-event)
  add_test(NAME bean-key-key-event COMMAND bean-key-key-event-test)

  add_executable(bean-key-engine-test tests/engine_test.cpp)
  target_compile_features(bean-key-engine-test PRIVATE cxx_std_20)
  target_compile_options(bean-key-engine-test PRIVATE -Wall -Wextra -Wpedantic -Werror)
  target_link_libraries(bean-key-engine-test PRIVATE bean-key-engine Threads::Threads)
  add_test(NAME bean-key-engine COMMAND bean-key-engine-test)
endif()
