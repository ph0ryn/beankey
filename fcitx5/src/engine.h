#pragma once

#include <fcitx/action.h>
#include <fcitx/inputcontextproperty.h>
#include <fcitx/inputmethodengine.h>

#include <chrono>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

#include "bean_key.pb.h"
#include "client.h"

namespace fcitx {

class Instance;
class InputContext;
class KeyEvent;

class BeanKeyEngine;

class BeanKeyState final : public InputContextProperty {
public:
  BeanKeyState(InputContext *inputContext, BeanKeyEngine *engine);
  ~BeanKeyState() override;

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
  bool pageCandidates(bean_key::v1::PageCandidates::Direction direction);
  bool commitComposition();
  bool send(bean_key::v1::Envelope request,
            const std::vector<bean_key::v1::CursorAction> &commitActions = {});
  bool apply(const bean_key::v1::Envelope &response,
             const std::vector<bean_key::v1::CursorAction> &commitActions);
  void showTypoCorrections(const bean_key::v1::TypoCorrectionResponse &response);
  void fillSurroundingText(bean_key::v1::SurroundingText *surrounding) const;
  void clearUi();
  void failSession();
  bean_key::v1::Envelope envelope();

  InputContext *inputContext_;
  BeanKeyEngine *engine_;
  std::string sessionId_;
  std::uint64_t nextRequestId_ = 1;
  bool started_ = false;
  std::int32_t selectedCandidate_ = -1;
  std::int32_t candidateWindowStart_ = 0;
  bool lmTypoAvailable_ = false;
  bool learningAvailable_ = false;
  bool learningWritable_ = false;
  std::vector<std::vector<bean_key::v1::CursorAction>> candidateActions_;
};

class BeanKeyEngine final : public InputMethodEngineV2 {
public:
  explicit BeanKeyEngine(Instance *instance);

  void activate(const InputMethodEntry &entry,
                InputContextEvent &event) override;
  void deactivate(const InputMethodEntry &entry,
                  InputContextEvent &event) override;
  void keyEvent(const InputMethodEntry &entry, KeyEvent &event) override;
  void reset(const InputMethodEntry &entry, InputContextEvent &event) override;

  BeanKeyState *state(InputContext *inputContext);
  bean_key::Client &client();
  bool ensureConnected();
  std::chrono::milliseconds requestTimeout() const;

private:
  Instance *instance_;
  bean_key::Client client_;
  FactoryFor<BeanKeyState> factory_;
  SimpleAction resetLearningAction_;
};

} // namespace fcitx
