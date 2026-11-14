// What the pages read and ask for. The work itself is in the Secpar app library
// (crates/secpar-app): one JSON request in, one JSON answer out, each call
// blocking, so every call here runs on a worker thread.
#pragma once

#include <QJsonObject>
#include <QObject>
#include <QVariantList>
#include <functional>

class Backend : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool loaded MEMBER m_loaded NOTIFY changed)
    Q_PROPERTY(bool signedIn MEMBER m_signedIn NOTIFY changed)
    Q_PROPERTY(QString handle MEMBER m_handle NOTIFY changed)
    Q_PROPERTY(bool busy MEMBER m_busy NOTIFY changed)
    Q_PROPERTY(QString message MEMBER m_message NOTIFY changed)
    Q_PROPERTY(QVariantList devices MEMBER m_devices NOTIFY changed)

public:
    explicit Backend(const QJsonObject &configuration, const QString &computerName, QObject *parent = nullptr);

    Q_INVOKABLE void refresh();
    Q_INVOKABLE void signIn(const QString &email, const QString &password);
    Q_INVOKABLE void signOut();
    Q_INVOKABLE void removeDevice(const QString &id);

signals:
    void changed();

private:
    void ask(const QJsonObject &request, std::function<void(const QJsonObject &)> then);
    void takeState(const QJsonObject &state);
    void loadDevices();
    static QString explain(const QJsonObject &answer);

    bool m_loaded = false;
    bool m_signedIn = false;
    bool m_busy = false;
    QString m_handle;
    QString m_computer;
    QString m_message;
    QVariantList m_devices;
};
