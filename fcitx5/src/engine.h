#pragma once

#include <fcitx/action.h>
#include <fcitx/inputcontextproperty.h>
#include <fcitx/inputmethodengine.h>

#include <chrono>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

#include "beankey.pb.h"
#include "client.h"

namespace fcitx {

class Instance;
class InputContext;
class KeyEvent;

class BeankeyEngine;

class BeankeyState final : public InputContextProperty {
public:
  BeankeyState(InputContext *inputContext, BeankeyEngine *engine);
  ~BeankeyState() override;

  bool focusIn();
  void focusOut(bool commitComposition = false);
  bool processKey(KeyEvent &event);
  bool selectCandidate(std::uint32_t index);
  bool selectTypoCorrection(std::uint32_t index);
  bool forgetCandidate(std::uint32_t index);
  bool resetLearning();
  bool requestTypoCorrections();
  bool learningAvailable() const;
  bool learningWritable() const;
  void reset();

private:
  bool start();
  bool pageCandidates(beankey::v1::PageCandidates::Direction direction);
  bool commitComposition();
  bool send(beankey::v1::Envelope request,
            const std::vector<beankey::v1::CursorAction> &commitActions = {});
  bool apply(const beankey::v1::Envelope &response,
             const std::vector<beankey::v1::CursorAction> &commitActions);
  void showTypoCorrections(const beankey::v1::TypoCorrectionResponse &response);
  void fillSurroundingText(beankey::v1::SurroundingText *surrounding) const;
  void clearUi();
  void failSession();
  beankey::v1::Envelope envelope();

  InputContext *inputContext_;
  BeankeyEngine *engine_;
  std::string sessionId_;
  std::uint64_t nextRequestId_ = 1;
  bool started_ = false;
  std::int32_t selectedCandidate_ = -1;
  bool lmTypoAvailable_ = false;
  bool learningAvailable_ = false;
  bool learningWritable_ = false;
  std::vector<std::vector<beankey::v1::CursorAction>> candidateActions_;
};

class BeankeyEngine final : public InputMethodEngineV2 {
public:
  explicit BeankeyEngine(Instance *instance);

  void activate(const InputMethodEntry &entry,
                InputContextEvent &event) override;
  void deactivate(const InputMethodEntry &entry,
                  InputContextEvent &event) override;
  void keyEvent(const InputMethodEntry &entry, KeyEvent &event) override;
  void reset(const InputMethodEntry &entry, InputContextEvent &event) override;

  BeankeyState *state(InputContext *inputContext);
  beankey::Client &client();
  bool ensureConnected();
  std::chrono::milliseconds requestTimeout() const;

private:
  Instance *instance_;
  beankey::Client client_;
  FactoryFor<BeankeyState> factory_;
  SimpleAction resetLearningAction_;
};

} // namespace fcitx
