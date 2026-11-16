// What the pages read and ask for. The work itself is in the Secpar app library
// (crates/secpar-app): one JSON request in, one JSON answer out, each call
// blocking, so every call here runs on a worker thread.
#pragma once

#include <QHash>
#include <QImage>
#include <QJsonObject>
#include <QObject>
#include <QQuickImageProvider>
#include <QUrl>
#include <QVariantList>
#include <QVariantMap>
#include <functional>

class QTimer;

class Backend;

// The stills the background part sent, for the computer tiles. A tile asks for
// "image://preview/<computer id>/<version>"; the version makes QML load the new
// picture instead of the one it already has.
class PreviewProvider : public QQuickImageProvider
{
public:
    explicit PreviewProvider(Backend *backend);
    QImage requestImage(const QString &id, QSize *size, const QSize &requested) override;

private:
    Backend *m_backend;
};

class Backend : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool loaded MEMBER m_loaded NOTIFY changed)
    Q_PROPERTY(bool signedIn MEMBER m_signedIn NOTIFY changed)
    Q_PROPERTY(bool background MEMBER m_background NOTIFY changed)
    Q_PROPERTY(QString handle MEMBER m_handle NOTIFY changed)
    Q_PROPERTY(bool busy MEMBER m_busy NOTIFY changed)
    Q_PROPERTY(QString message MEMBER m_message NOTIFY changed)
    Q_PROPERTY(QVariantList devices MEMBER m_devices NOTIFY changed)
    // Friends, and requests that are waiting for this account's answer.
    Q_PROPERTY(QVariantList friends MEMBER m_friends NOTIFY changed)
    Q_PROPERTY(QVariantList friendRequests MEMBER m_friendRequests NOTIFY changed)
    // The computer this one has a tunnel to, and the one it is trying to reach.
    Q_PROPERTY(QString connectedId MEMBER m_connectedId NOTIFY changed)
    Q_PROPERTY(QString connectingId MEMBER m_connectingId NOTIFY changed)
    // True while the stream window for the connected computer is running.
    Q_PROPERTY(bool streaming MEMBER m_streaming NOTIFY changed)
    // How far a file being sent to that computer has got, and a file that just
    // arrived here from another computer. Both are empty when there is nothing
    // to show.
    Q_PROPERTY(QString transferText MEMBER m_transferText NOTIFY changed)
    Q_PROPERTY(QString receivedText MEMBER m_receivedText NOTIFY changed)
    // A still of a computer's screen, by computer id, as an image URL. Only a
    // computer this one has a tunnel to can send one.
    Q_PROPERTY(QVariantMap previews MEMBER m_previews NOTIFY changed)
    // Settings → Connecting, and the hosting state Settings → Hosting shows.
    Q_PROPERTY(QVariantMap preferences MEMBER m_preferences NOTIFY changed)
    Q_PROPERTY(bool hosting MEMBER m_hosting NOTIFY changed)
    Q_PROPERTY(int allowedDevices MEMBER m_allowedDevices NOTIFY changed)
    Q_PROPERTY(QString accessList MEMBER m_accessList NOTIFY changed)
    // Settings → About: what the release feed last said, and the version it
    // offers. `updateConfirming` is the step before installing, which ends a
    // stream, so the person is asked first.
    Q_PROPERTY(QString updateText MEMBER m_updateText NOTIFY changed)
    Q_PROPERTY(QString updateVersion MEMBER m_updateVersion NOTIFY changed)
    Q_PROPERTY(bool updateAvailable MEMBER m_updateAvailable NOTIFY changed)
    Q_PROPERTY(bool updateConfirming MEMBER m_updateConfirming NOTIFY changed)
    Q_PROPERTY(bool installing MEMBER m_installing NOTIFY changed)

public:
    explicit Backend(const QJsonObject &configuration, const QString &computerName, QObject *parent = nullptr);

    Q_INVOKABLE void refresh();
    Q_INVOKABLE void signIn(const QString &email, const QString &password);
    Q_INVOKABLE void signOut();
    Q_INVOKABLE void removeDevice(const QString &id);
    Q_INVOKABLE void removeDuplicateDevices();
    Q_INVOKABLE void acceptFriend(const QString &id);
    Q_INVOKABLE void removeFriend(const QString &id);
    /// Asks the account named `handle` (with or without the @) to be friends.
    Q_INVOKABLE void addFriend(const QString &handle);
    /// Changes what one friend may do. `change` holds any of "input" ("view" or
    /// "full"), "connect" ("ask" or "always"), "keyboard" and "mouse".
    Q_INVOKABLE void saveFriendSettings(const QString &id, const QVariantMap &change);
    Q_INVOKABLE void connectTo(const QString &id);
    Q_INVOKABLE void disconnect();
    Q_INVOKABLE void sendFile(const QUrl &file);
    /// Whether the Computers page is on screen. Stills are only asked for while
    /// it is, so a hidden page costs the other computer nothing.
    Q_INVOKABLE void watchPreviews(bool on);
    /// The still behind an "image://preview/..." address.
    QImage previewImage(const QString &id) const;
    Q_INVOKABLE void setPreference(const QString &key, const QVariant &value);
    Q_INVOKABLE void checkForUpdates();
    Q_INVOKABLE void installUpdate();
    Q_INVOKABLE void cancelUpdate();

signals:
    void changed();
    // The new version is in place and this window must go so it can start.
    void restartForUpdate();
    /// Whether the last friend request reached the account service.
    void friendRequestSent(bool sent);

private:
    void ask(const QJsonObject &request, std::function<void(const QJsonObject &)> then, bool busy = true);
    void takeState(const QJsonObject &state);
    void takeTransfers(const QJsonObject &answer);
    void loadDevices();
    void loadFriends();
    void takeFriendSettings(const QString &id, const QJsonObject &answer);
    void pollState();
    void pollTransfers();
    void requestPreviews();
    void askNextPreview();
    void shutdown();
    static QString explain(const QJsonObject &answer);

    bool m_loaded = false;
    bool m_signedIn = false;
    // Whether the background part answered: until it does, this computer's own
    // identity is unknown, so no computer list is shown at all.
    bool m_background = false;
    bool m_busy = false;
    bool m_polling = false;
    bool m_pollingTransfers = false;
    QString m_handle;
    QString m_computer;
    QString m_message;
    QString m_connectedId;
    QString m_connectingId;
    bool m_streaming = false;
    // The file being sent, and the last file that arrived here.
    QString m_transferText;
    QString m_receivedText;
    QString m_receivedKey;
    // The stills the connected computer sent, and the picture behind each one.
    QVariantMap m_previews;
    QHash<QString, QImage> m_previewImages;
    // The computers still to be asked for a still in this turn: one peek at a
    // time, so a page of tiles does not open a tunnel for each of them at once.
    QVariantList m_previewQueue;
    bool m_previewWatch = false;
    qint64 m_previewAsked = 0;
    int m_previewVersion = 0;
    QVariantMap m_preferences;
    bool m_hosting = false;
    int m_allowedDevices = 0;
    QString m_accessList;
    QString m_updateText;
    QString m_updateVersion;
    bool m_updateAvailable = false;
    bool m_updateConfirming = false;
    bool m_installing = false;
    QVariantList m_devices;
    QVariantList m_friends;
    QVariantList m_friendRequests;
    QTimer *m_watch = nullptr;
    QTimer *m_transfers = nullptr;
    QTimer *m_preview = nullptr;
};
