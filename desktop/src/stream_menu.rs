use super::*;
use egui::{Align2,FontId,Pos2,Rect,Response,Sense,Stroke,Vec2};

use crate::icons::{paint as icon,Icon};
const TEXT:Color32=Color32::from_rgb(233,235,237);
const MUTED:Color32=Color32::from_rgb(148,153,158);
const RED:Color32=Color32::from_rgb(242,105,94);

fn row(ui:&mut egui::Ui,kind:Icon,label:&str,value:&str,arrow:bool,checked:bool,danger:bool)->Response {
    let (rect,response)=ui.allocate_exact_size(Vec2::new(ui.available_width(),34.),Sense::click());
    if response.hovered() || response.has_focus(){ui.painter().rect_filled(rect,3.,Color32::from_white_alpha(12));}
    let color=if danger {RED}else{TEXT};
    icon(ui,kind,Rect::from_min_size(rect.min+egui::vec2(9.,7.),egui::vec2(20.,20.)),color);
    ui.painter().text(Pos2::new(rect.left()+39.,rect.center().y),Align2::LEFT_CENTER,label,FontId::proportional(14.),color);
    let right=rect.right()-12.;
    if checked || arrow {
        icon(ui,if checked {Icon::Check}else{Icon::Chevron},Rect::from_center_size(
            egui::pos2(right-6.,rect.center().y),egui::vec2(18.,18.)),if checked {design::ACCENT}else{MUTED});
    }
    if !value.is_empty(){ui.painter().text(egui::pos2(right-if arrow||checked {18.}else{0.},rect.center().y),Align2::RIGHT_CENTER,value,FontId::proportional(12.),MUTED);}
    response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,ui.is_enabled(),label));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl App {
    pub(super) fn stream_menu(&mut self,ctx:&egui::Context) {
        let button=egui::Area::new(egui::Id::new("stream-menu-button")).order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(12.,12.)).show(ctx,|ui|{
                ui.spacing_mut().button_padding=egui::vec2(8.,8.);
                if ui.add_sized([40.,40.],egui::Button::image(egui::Image::new((self.mark.id(),egui::vec2(24.,24.))))
                    .fill(Color32::from_rgba_unmultiplied(25,27,29,175)).stroke(Stroke::NONE).corner_radius(6))
                    .on_hover_text("Stream menu").clicked(){self.stream_menu_open=!self.stream_menu_open;self.menu_page=0;}
            });
        self.stream_menu_button_bounds=Some(button.response.rect);
        if !self.stream_menu_open {
            self.stream_menu_bounds=None;
            if self.show_stream_stats && self.texture.is_some() {
                egui::Area::new(egui::Id::new("stream-stats")).order(egui::Order::Tooltip).interactable(false)
                    .anchor(egui::Align2::RIGHT_TOP,egui::vec2(-12.,12.)).show(ctx,|ui|{
                        egui::Frame::new().fill(Color32::from_rgba_unmultiplied(15,17,19,200)).corner_radius(4).inner_margin(9).show(ui,|ui|{
                            ui.set_max_width((ctx.content_rect().width()-90.).clamp(140.,430.));
                            ui.small(format!("{} × {} · {} fps · {}",self.dimensions[0],self.dimensions[1],self.stream_fps,self.status));
                            if let Some((rtt,mbps,fps))=self.stream_stats {
                                let latency=rtt.map(|n|format!("RTT {n} ms · ")).unwrap_or_default();
                                ui.small(format!("{latency}{mbps:.1} Mbps · {fps:.1} received fps"));
                            }
                            if let Some(decoder)=&self.client.active_decoder {ui.small(decoder);}
                        });
                    });
            }
            return;
        }
        let panel=egui::Area::new(egui::Id::new("stream-menu-panel")).order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(60.,12.)).show(ctx,|ui|{
                egui::Frame::new().fill(Color32::from_rgba_unmultiplied(25,27,29,246)).corner_radius(5).inner_margin(6).show(ui,|ui|{
                    ui.set_width(250.);ui.spacing_mut().item_spacing.y=0.;
                    if self.menu_page==0 {
                        if row(ui,if self.fullscreen {Icon::Windowed}else{Icon::Window},if self.fullscreen {"Window"}else{"Fullscreen"},"Alt+F",false,false,false).clicked(){self.set_fullscreen(ctx,!self.fullscreen);}
                        if row(ui,Icon::Decoder,"Decoder","",true,false,false).clicked(){self.menu_page=1;}
                        if self.displays.len()>1 && row(ui,Icon::Display,"Switch display","",true,false,false).clicked(){self.menu_page=2;}
                        if row(ui,Icon::Clipboard,"Share clipboard","",false,self.client.clipboard,false).clicked(){
                            self.client.clipboard=!self.client.clipboard;self.client.save();self.options.clipboard=self.client.clipboard;
                            self.clipboard=self.client.clipboard.then(secpar_clipboard::Bridge::start);self.send(Command::ClipboardSharing(self.client.clipboard));
                        }
                        if row(ui,Icon::Stats,"Stream stats","",false,self.show_stream_stats,false).clicked(){self.show_stream_stats=!self.show_stream_stats;self.stream_menu_open=false;}
                        if row(ui,Icon::Audio,"Audio","",true,false,false).clicked(){self.menu_page=3;}
                        if row(ui,Icon::Help,"Help","",false,false,false).clicked(){let _=open::that("https://secpar.win/help/#stream-menu");}
                        ui.add_space(5.);ui.separator();ui.add_space(5.);
                        if row(ui,Icon::Exit,"Disconnect","",false,false,true).clicked(){self.capture(ctx,false);self.send(Command::Stop);self.stream_menu_open=false;}
                        ui.add_space(5.);ui.horizontal(|ui|{ui.add_space(10.);ui.label(egui::RichText::new(concat!("Secpar ",env!("CARGO_PKG_VERSION"))).size(10.).color(MUTED));});
                    } else {
                        let page=self.menu_page;
                        let title=match page {1=>"Decoder",2=>"Displays",_=>"Audio"};
                        if row(ui,Icon::Back,title,"",false,false,false).clicked(){self.menu_page=0;}
                        ui.separator();
                        if page==1 {
                            for (preference,label) in [(video::Preference::Automatic,"Automatic"),(video::Preference::Software,"Software"),(video::Preference::Vaapi,"VA-API")] {
                                if preference==video::Preference::Vaapi && video::hardware_available(ctx)!=Some(true){continue;}
                                if row(ui,Icon::Decoder,label,"",false,self.client.decoder==preference,false).clicked(){
                                    if self.client.decoder!=preference {self.client.decoder=preference;self.client.save();self.decoder=None;self.client.active_decoder=None;self.send(Command::Keyframe);}
                                    self.menu_page=0;
                                }
                            }
                            if let Some(active)=&self.client.active_decoder {ui.add_space(8.);ui.label(egui::RichText::new(active).size(11.).color(MUTED));}
                        } else if page==2 {
                            let mut selected=None;
                            egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-90.).max(80.)).show(ui,|ui|{
                                for (id,name) in &self.displays {if row(ui,Icon::Display,name,"",false,self.display==*id,false).clicked(){selected=Some(id.clone());}}
                            });
                            if let Some(id)=selected {self.display=id.clone();self.send(Command::Display(id));self.menu_page=0;}
                        } else {
                            egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-130.).max(80.)).show(ui,|ui|{
                                ui.add_space(8.);
                                if let Some(note)=&self.notice {ui.label(egui::RichText::new(note).size(12.).color(RED));}
                                if let Some(error)=&self.audio_error {ui.label(error);}
                                if self.audio.is_none() && self.audio_error.is_none() {ui.label(egui::RichText::new("Host audio is off in client settings").size(12.).color(MUTED));}
                                if let Some(audio)=&self.audio {let details=audio.details();ui.label(egui::RichText::new(&details).size(12.).color(TEXT));if row(ui,Icon::Clipboard,"Copy details","",false,false,false).clicked(){ui.ctx().copy_text(details);}}
                                if self.client.audio && row(ui,Icon::Audio,"Reopen output","",false,false,false).clicked(){self.open_audio();}
                            });
                        }
                    }
                });
            });
        self.stream_menu_bounds=Some(panel.response.rect);
    }
}
