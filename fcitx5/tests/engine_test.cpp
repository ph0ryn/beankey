#include "engine.h"

#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <fcitx-utils/capabilityflags.h>
#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx/candidatelist.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputcontextmanager.h>
#include <fcitx/inputmethodentry.h>
#include <fcitx/inputpanel.h>
#include <fcitx/instance.h>

#include <array>
#include <concepts>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <thread>
#include <utility>

#include "beankey.pb.h"

namespace {

bool readAll(int socket, void *data, std::size_t size) {
  auto *current = static_cast<std::uint8_t *>(data);
  while (size > 0) {
    const auto count = read(socket, current, size);
    if (count <= 0) {
      return false;
    }
    current += count;
    size -= static_cast<std::size_t>(count);
  }
  return true;
}

bool writeAll(int socket, const void *data, std::size_t size) {
  const auto *current = static_cast<const std::uint8_t *>(data);
  while (size > 0) {
    const auto count = write(socket, current, size);
    if (count <= 0) {
      return false;
    }
    current += count;
    size -= static_cast<std::size_t>(count);
  }
  return true;
}

std::string readFrame(int socket) {
  std::uint32_t size = 0;
  for (unsigned int index = 0; index < 5; ++index) {
    std::uint8_t byte = 0;
    if (!readAll(socket, &byte, 1)) {
      return {};
    }
    size |= static_cast<std::uint32_t>(byte & 0x7fU) << (index * 7U);
    if ((byte & 0x80U) == 0) {
      std::string payload(size, '\0');
      return readAll(socket, payload.data(), payload.size()) ? payload
                                                             : std::string{};
    }
  }
  return {};
}

bool writeFrame(int socket, const std::string &payload) {
  std::array<std::uint8_t, 5> prefix{};
  auto size = static_cast<std::uint32_t>(payload.size());
  std::size_t count = 0;
  do {
    auto byte = static_cast<std::uint8_t>(size & 0x7fU);
    size >>= 7U;
    if (size != 0) {
      byte |= 0x80U;
    }
    prefix[count++] = byte;
  } while (size != 0);
  return writeAll(socket, prefix.data(), count) &&
         writeAll(socket, payload.data(), payload.size());
}

class TestInputContext final : public fcitx::InputContext {
public:
  explicit TestInputContext(fcitx::InputContextManager &manager)
      : InputContext(manager, "beankey-engine-test") {
    created();
  }

  ~TestInputContext() override { destroy(); }

  const char *frontend() const override { return "beankey-test"; }

  const std::string &committed() const { return committed_; }

protected:
  void commitStringImpl(const std::string &text) override {
    committed_ += text;
  }
  void deleteSurroundingTextImpl(int, unsigned int) override {}
  void forwardKeyImpl(const fcitx::ForwardKeyEvent &) override {}
  void updatePreeditImpl() override {}

private:
  std::string committed_;
};

template <typename State>
concept CanForgetCandidate = requires(State &state) {
  { state.forgetCandidate(std::uint32_t{}) } -> std::same_as<bool>;
};

template <typename State>
concept CanResetLearning = requires(State &state) {
  { state.resetLearning() } -> std::same_as<bool>;
};

template <typename State>
concept CanRequestTypoCorrections = requires(State &state) {
  { state.requestTypoCorrections() } -> std::same_as<bool>;
};

template <typename State> bool forgetCandidate(State &state) {
  if constexpr (CanForgetCandidate<State>) {
    return state.forgetCandidate(0);
  }
  return false;
}

template <typename State> bool resetLearning(State &state) {
  if constexpr (CanResetLearning<State>) {
    return state.resetLearning();
  }
  return false;
}

template <typename State> bool requestTypoCorrections(State &state) {
  if constexpr (CanRequestTypoCorrections<State>) {
    return state.requestTypoCorrections();
  }
  return false;
}

bool report(bool condition, const char *message) {
  if (!condition) {
    std::cerr << message << '\n';
  }
  return condition;
}

} // namespace

int main() {
  std::array<char, 64> directoryTemplate{};
  std::strcpy(directoryTemplate.data(), "/tmp/beankey-engine-test.XXXXXX");
  const char *runtimeDirectory = mkdtemp(directoryTemplate.data());
  if (!report(runtimeDirectory != nullptr,
              "failed to create runtime directory")) {
    return 1;
  }
  const std::string beankeyDirectory =
      std::string(runtimeDirectory) + "/beankey";
  if (!report(mkdir(beankeyDirectory.c_str(), 0700) == 0,
              "failed to create beankey runtime directory")) {
    return 1;
  }
  const std::string socketPath = beankeyDirectory + "/daemon.sock";
  if (!report(setenv("XDG_RUNTIME_DIR", runtimeDirectory, 1) == 0,
              "failed to set runtime directory")) {
    return 1;
  }

  const int listener = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
  sockaddr_un address{};
  address.sun_family = AF_UNIX;
  std::memcpy(address.sun_path, socketPath.c_str(), socketPath.size() + 1);
  if (!report(listener >= 0, "failed to create daemon socket") ||
      !report(bind(listener, reinterpret_cast<const sockaddr *>(&address),
                   sizeof(address)) == 0,
              "failed to bind daemon socket") ||
      !report(listen(listener, 1) == 0, "failed to listen on daemon socket")) {
    return 1;
  }

  bool serverValid = true;
  std::thread server([listener, &serverValid] {
    const int connection = accept4(listener, nullptr, nullptr, SOCK_CLOEXEC);
    if (connection < 0) {
      serverValid = false;
      close(listener);
      return;
    }
    enum class ExpectedRequest { Start, Key, Forget, ResetLearning, Typo };
    int keyIndex = 0;
    const auto exchange = [&](ExpectedRequest expected) {
      beankey::v1::Envelope request;
      if (!request.ParseFromString(readFrame(connection))) {
        return false;
      }
      const bool expectedPayload = [&] {
        switch (expected) {
        case ExpectedRequest::Start:
          return request.has_start_session();
        case ExpectedRequest::Key:
          return request.has_key_event();
        case ExpectedRequest::Forget:
          return request.has_forget_candidate();
        case ExpectedRequest::ResetLearning:
          return request.has_reset_learning();
        case ExpectedRequest::Typo:
          return request.has_request_typo_corrections();
        }
        return false;
      }();
      if (!expectedPayload) {
        return false;
      }

      beankey::v1::Envelope response;
      response.set_protocol_version(request.protocol_version());
      response.set_request_id(request.request_id());
      response.set_session_id(request.session_id());
      if (expected == ExpectedRequest::Typo) {
        auto *candidate =
            response.mutable_typo_correction_response()->add_candidates();
        candidate->set_corrected_input("かな");
        candidate->set_converted_text("仮名");
      } else {
        auto *state = response.mutable_state_response();
        if (expected == ExpectedRequest::Start) {
          state->set_consumed(true);
        } else if (expected == ExpectedRequest::Key && keyIndex == 0) {
          state->set_consumed(true);
          state->set_preedit("かな");
          state->set_preedit_cursor(2);
          state->set_selected_candidate(0);
          state->add_candidates()->set_text("司会");
        } else if (expected == ExpectedRequest::Key && keyIndex == 1) {
          state->set_consumed(true);
          state->set_commit("司会");
          state->set_reset(true);
        } else if (expected != ExpectedRequest::Key) {
          state->set_consumed(true);
        }
      }
      if (expected == ExpectedRequest::Key) {
        ++keyIndex;
      }

      std::string payload;
      return response.SerializeToString(&payload) &&
             writeFrame(connection, payload);
    };

    serverValid = exchange(ExpectedRequest::Start) && serverValid;
    serverValid = exchange(ExpectedRequest::Key) && serverValid;
    if constexpr (CanForgetCandidate<fcitx::BeankeyState>) {
      serverValid = exchange(ExpectedRequest::Forget) && serverValid;
    }
    serverValid = exchange(ExpectedRequest::Key) && serverValid;
    serverValid = exchange(ExpectedRequest::Key) && serverValid;
    if constexpr (CanResetLearning<fcitx::BeankeyState>) {
      serverValid = exchange(ExpectedRequest::ResetLearning) && serverValid;
    }
    if constexpr (CanRequestTypoCorrections<fcitx::BeankeyState>) {
      serverValid = exchange(ExpectedRequest::Typo) && serverValid;
    }
    close(connection);
    close(listener);
  });

  bool valid = true;
  {
    char argument0[] = "beankey-engine-test";
    char argument1[] = "--disable=all";
    char *arguments[] = {argument0, argument1};
    fcitx::Instance instance(2, arguments);
    valid =
        report(instance.initialized(), "Fcitx instance did not initialize") &&
        valid;
    fcitx::BeankeyEngine engine(&instance);
    TestInputContext inputContext(instance.inputContextManager());
    inputContext.setCapabilityFlags(fcitx::CapabilityFlag::Preedit);
    fcitx::InputMethodEntry entry("beankey", "beankey", "ja", "beankey");

    fcitx::KeyEvent printable(&inputContext, fcitx::Key(FcitxKey_a));
    engine.keyEvent(entry, printable);
    valid = report(printable.accepted(),
                   "consumed printable key was not accepted") &&
            valid;
    valid =
        report(inputContext.inputPanel().clientPreedit().toString() == "かな",
               "daemon preedit did not reach Fcitx") &&
        valid;
    const auto candidates = inputContext.inputPanel().candidateList();
    valid = report(candidates && candidates->size() == 1,
                   "daemon candidates did not reach Fcitx") &&
            valid;
    auto *actionable = candidates ? candidates->toActionable() : nullptr;
    valid = report(actionable != nullptr,
                   "candidate forgetting is not reachable from Fcitx") &&
            valid;
    const bool hasForgetAction =
        actionable != nullptr &&
        actionable->hasAction(candidates->candidate(0));
    valid = report(hasForgetAction, "candidate has no Fcitx forget action") &&
            valid;
    auto *state = engine.state(&inputContext);
    const bool canForget = CanForgetCandidate<fcitx::BeankeyState>;
    valid =
        report(canForget, "addon has no ForgetCandidate request entry point") &&
        valid;
    if (canForget) {
      if (hasForgetAction) {
        const auto actions =
            actionable->candidateActions(candidates->candidate(0));
        valid = report(!actions.empty(), "candidate forget action is empty") &&
                valid;
        if (!actions.empty()) {
          actionable->triggerAction(candidates->candidate(0),
                                    actions.front().id());
        }
      } else {
        valid = report(forgetCandidate(*state),
                       "addon did not send ForgetCandidate") &&
                valid;
      }
    }

    fcitx::KeyEvent enter(&inputContext, fcitx::Key(FcitxKey_Return));
    engine.keyEvent(entry, enter);
    valid =
        report(enter.accepted(), "consumed Enter was not accepted") && valid;
    valid = report(inputContext.committed() == "司会",
                   "daemon commit did not reach Fcitx") &&
            valid;
    valid = report(inputContext.inputPanel().clientPreedit().empty(),
                   "committed preedit was not cleared") &&
            valid;

    fcitx::KeyEvent tab(&inputContext, fcitx::Key(FcitxKey_Tab));
    engine.keyEvent(entry, tab);
    valid = report(!tab.accepted(), "unconsumed Tab was accepted") && valid;

    const bool canResetLearning = CanResetLearning<fcitx::BeankeyState>;
    valid = report(canResetLearning,
                   "addon has no ResetLearning request entry point") &&
            valid;
    if (canResetLearning) {
      valid =
          report(resetLearning(*state), "addon did not send ResetLearning") &&
          valid;
    }
    const bool canRequestTypo = CanRequestTypoCorrections<fcitx::BeankeyState>;
    valid = report(canRequestTypo,
                   "addon has no RequestTypoCorrections entry point") &&
            valid;
    if (canRequestTypo) {
      valid = report(requestTypoCorrections(*state),
                     "addon did not send RequestTypoCorrections") &&
              valid;
    }
  }

  server.join();
  valid = report(serverValid, "addon sent an unexpected IPC request") && valid;
  valid = report(unlink(socketPath.c_str()) == 0,
                 "failed to remove daemon socket") &&
          valid;
  valid = report(rmdir(beankeyDirectory.c_str()) == 0,
                 "failed to remove beankey runtime directory") &&
          valid;
  valid = report(rmdir(runtimeDirectory) == 0,
                 "failed to remove runtime directory") &&
          valid;
  return valid ? 0 : 1;
}
